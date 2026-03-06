//! Snapshot modules for Phase 2 Checkpoint implementation
//!
//! This module includes:
//! - `checkpoint` - SurrealKV checkpoint creation
//! - `compression` - tar + zstd compression
//! - `restore` - decompression and recovery
//! - `metadata` - checkpoint metadata management

use crate::error::{Error, Result};
use crate::state::SnapshotMetaState;
use crate::types::{DeltaEntry, DeltaMetadata, RaftTypeConfig, SnapshotFormat};
use openraft::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use surrealkv::Tree;

pub mod checkpoint;
pub mod compression;
pub mod consistency;
pub mod delta;
pub mod metadata;
pub mod restore;

pub use checkpoint::CheckpointBuilder;
pub use compression::SnapshotCompressor;
pub use consistency::{
    parse_sequence_from_checkpoint_manifest, resolve_sequence_number,
    PersistedKeySequenceNumberReader, SequenceNumberReader,
};
pub use delta::DeltaSnapshotCodec;
pub use metadata::CheckpointMetadata;
pub use restore::SnapshotRestorer;

const DELTA_ENTRY_THRESHOLD: usize = 1000;
const DELTA_MAX_BYTES: u64 = 5 * 1024 * 1024;
const DELTA_MAX_AGE_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotEnvelope {
    metadata: DeltaMetadata,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum LegacySnapshotFormatV1 {
    Full {
        checkpoint_meta: CheckpointMetadata,
    },
    Delta {
        base_index: u64,
        entries: Vec<DeltaEntry>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyDeltaMetadataV1 {
    format: LegacySnapshotFormatV1,
    size_bytes: u64,
    created_at: u64,
    file_path: Option<String>,
    version: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacySnapshotEnvelopeV1 {
    metadata: LegacyDeltaMetadataV1,
    payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum DecodedSnapshotPayload {
    Full(DeltaMetadata),
    Delta {
        metadata: DeltaMetadata,
        entries: Vec<DeltaEntry>,
    },
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn should_build_delta(
    snapshot_state: &SnapshotMetaState,
    entries: usize,
    size_bytes: u64,
    now: u64,
) -> bool {
    // Delta snapshots require a full checkpoint baseline.
    if snapshot_state.last_checkpoint.is_none() {
        return false;
    }

    if entries == 0 || size_bytes > DELTA_MAX_BYTES {
        return false;
    }

    let by_entry_threshold = entries >= DELTA_ENTRY_THRESHOLD;
    let within_time_window = snapshot_state
        .last_checkpoint
        .as_ref()
        .map(|c| now.saturating_sub(c.created_at) <= DELTA_MAX_AGE_SECS)
        .unwrap_or(false);

    by_entry_threshold || within_time_window
}

/// Decode snapshot payload envelope.
pub fn decode_snapshot_payload(payload: &[u8]) -> Result<Option<DecodedSnapshotPayload>> {
    if let Ok(envelope) = postcard::from_bytes::<SnapshotEnvelope>(payload) {
        return match envelope.metadata.format {
            SnapshotFormat::Delta { .. } => {
                let entries = DeltaSnapshotCodec::decode_entries(&envelope.payload)?;
                Ok(Some(DecodedSnapshotPayload::Delta {
                    metadata: envelope.metadata,
                    entries,
                }))
            }
            SnapshotFormat::Full { .. } => {
                Ok(Some(DecodedSnapshotPayload::Full(envelope.metadata)))
            }
        };
    }

    // Backward compatibility: read v1 format (metadata carried entries).
    let legacy: LegacySnapshotEnvelopeV1 = match postcard::from_bytes(payload) {
        Ok(x) => x,
        Err(_) => return Ok(None),
    };

    match legacy.metadata.format {
        LegacySnapshotFormatV1::Full { checkpoint_meta } => {
            let metadata = DeltaMetadata {
                format: SnapshotFormat::Full { checkpoint_meta },
                size_bytes: legacy.metadata.size_bytes,
                created_at: legacy.metadata.created_at,
                file_path: legacy.metadata.file_path,
                version: 2,
            };
            Ok(Some(DecodedSnapshotPayload::Full(metadata)))
        }
        LegacySnapshotFormatV1::Delta {
            base_index,
            entries,
        } => {
            let metadata = DeltaMetadata {
                format: SnapshotFormat::Delta {
                    base_index,
                    entry_count: entries.len() as u64,
                },
                size_bytes: legacy.metadata.size_bytes,
                created_at: legacy.metadata.created_at,
                file_path: legacy.metadata.file_path,
                version: 2,
            };
            Ok(Some(DecodedSnapshotPayload::Delta { metadata, entries }))
        }
    }
}

/// Decode and return delta entries if this payload is an envelope containing a delta snapshot.
pub fn decode_delta_entries_from_payload(payload: &[u8]) -> Result<Option<Vec<DeltaEntry>>> {
    match decode_snapshot_payload(payload)? {
        Some(DecodedSnapshotPayload::Delta { entries, .. }) => Ok(Some(entries)),
        _ => Ok(None),
    }
}

/// Runtime-injected snapshot metadata baseline.
#[derive(Debug, Clone)]
pub struct SnapshotBuildConfig {
    pub node_id: u64,
    pub node_addr: String,
    pub current_term: u64,
}

impl Default for SnapshotBuildConfig {
    fn default() -> Self {
        Self {
            node_id: 1,
            node_addr: "127.0.0.1:50051".to_string(),
            current_term: 0,
        }
    }
}

impl SnapshotBuildConfig {
    /// Build config from startup environment variables.
    /// - `NODE_ID` (default: 1)
    /// - `LISTEN_ADDR` (default: 127.0.0.1:50051)
    /// - `CURRENT_TERM` (default: 0)
    /// - `MEMBERSHIP_JSON` (JSON array: [{"node_id":1,"addr":"127.0.0.1:50051"},...])
    pub fn from_env() -> Self {
        // Try JSON membership first (priority).
        if let Ok(json_str) = std::env::var("MEMBERSHIP_JSON") {
            if let Ok(parsed) = Self::parse_membership_json(&json_str) {
                return parsed;
            }
        }

        // Fallback to individual env vars.
        let node_id = std::env::var("NODE_ID")
            .ok()
            .and_then(|x| x.parse::<u64>().ok())
            .unwrap_or(1);
        let node_addr =
            std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string());
        let current_term = std::env::var("CURRENT_TERM")
            .ok()
            .and_then(|x| x.parse::<u64>().ok())
            .unwrap_or(0);

        Self {
            node_id,
            node_addr,
            current_term,
        }
    }

    /// Parse membership from JSON format.
    /// Expected: `[{"node_id":1,"addr":"127.0.0.1:50051"},...]`
    /// For single-node: takes first entry.
    fn parse_membership_json(json_str: &str) -> Result<Self> {
        #[derive(serde::Deserialize)]
        struct MemberEntry {
            node_id: u64,
            addr: String,
        }

        let entries: Vec<MemberEntry> = serde_json::from_str(json_str)
            .map_err(|e| Error::Snapshot(format!("parse membership json failed: {}", e)))?;

        if entries.is_empty() {
            return Err(Error::Snapshot(
                "membership json contains no nodes".to_string(),
            ));
        }

        // For Phase 2 single-node: use first entry.
        let first = &entries[0];
        Ok(Self {
            node_id: first.node_id,
            node_addr: first.addr.clone(),
            current_term: 0, // Term not included in membership json.
        })
    }
}

/// RaftSnapshotBuilder - responsible for building snapshots from current state.
pub struct RaftSnapshotBuilder {
    tree: Arc<Tree>,
    build_cfg: SnapshotBuildConfig,
}

impl RaftSnapshotBuilder {
    /// Create a new snapshot builder using startup env as injected config.
    pub fn new(tree: Arc<Tree>) -> Self {
        Self {
            tree,
            build_cfg: SnapshotBuildConfig::from_env(),
        }
    }

    /// Create a new snapshot builder with explicit injected config.
    pub fn new_with_config(tree: Arc<Tree>, build_cfg: SnapshotBuildConfig) -> Self {
        Self { tree, build_cfg }
    }

    /// Build snapshot with automatic full/delta selection.
    pub async fn build_snapshot_with_delta(
        &mut self,
        applied_index: u64,
        snapshot_state: &SnapshotMetaState,
        delta_entries: Vec<DeltaEntry>,
    ) -> Result<Snapshot<RaftTypeConfig>> {
        let now = now_secs();
        let compressed_delta = DeltaSnapshotCodec::encode_entries(&delta_entries)?;
        let use_delta = should_build_delta(
            snapshot_state,
            delta_entries.len(),
            compressed_delta.len() as u64,
            now,
        );

        if use_delta {
            let delta_path = DeltaSnapshotCodec::persist_to_temp_file(
                &PathBuf::from("target/tmp/deltas"),
                &compressed_delta,
                &format!("{}", applied_index),
            )
            .await?;

            let base_index = snapshot_state
                .last_checkpoint
                .as_ref()
                .map(|c| c.checkpoint_index)
                .unwrap_or(0);

            let metadata = DeltaMetadata::new(
                SnapshotFormat::Delta {
                    base_index,
                    entry_count: delta_entries.len() as u64,
                },
                compressed_delta.len() as u64,
                now,
            )
            .with_file_path(delta_path.display().to_string());

            let envelope = SnapshotEnvelope {
                metadata,
                payload: compressed_delta,
            };
            let bytes = postcard::to_stdvec(&envelope)
                .map_err(|e| Error::Snapshot(format!("encode delta envelope failed: {}", e)))?;

            let mut snapshot_meta = openraft::SnapshotMeta::<RaftTypeConfig>::default();
            snapshot_meta.last_log_id = Some(openraft::LogId::<RaftTypeConfig>::new_term_index(
                self.build_cfg.current_term,
                applied_index,
            ));
            snapshot_meta.snapshot_id = format!("delta-{}-{}", applied_index, now);

            // Keep membership baseline consistent with full snapshots.
            let mut voters = BTreeSet::new();
            voters.insert(self.build_cfg.node_id);
            let mut nodes = BTreeMap::new();
            nodes.insert(
                self.build_cfg.node_id,
                openraft::BasicNode::new(&self.build_cfg.node_addr),
            );
            let membership = openraft::Membership::new(vec![voters], nodes)
                .map_err(|e| Error::Snapshot(format!("membership build failed: {}", e)))?;
            snapshot_meta.last_membership = openraft::StoredMembership::new(None, membership);

            return Ok(Snapshot {
                meta: snapshot_meta,
                snapshot: Cursor::new(bytes),
            });
        }

        self.build_snapshot(applied_index).await
    }

    /// Build snapshot through the checkpoint + tar.zst pipeline.
    pub async fn build_snapshot(&mut self, applied_index: u64) -> Result<Snapshot<RaftTypeConfig>> {
        let checkpoint_base = PathBuf::from("target/checkpoints");
        let checkpoint_builder = CheckpointBuilder::new(checkpoint_base.clone());

        // Term is not plumbed through storage yet; keep 0 for current Week 2 scaffold.
        let meta = checkpoint_builder
            .create(
                self.tree.clone(),
                applied_index,
                self.build_cfg.current_term,
            )
            .await?;

        let timestamp = now_secs();
        let out = PathBuf::from("target/snapshots").join(format!("snapshot-{}.tar.zst", timestamp));

        let compressed_size = SnapshotCompressor::new()
            .compress(&meta, out.clone())
            .await?;
        let bytes = tokio::fs::read(&out).await?;

        let full_meta = meta.with_compressed_size(compressed_size);
        let envelope = SnapshotEnvelope {
            metadata: DeltaMetadata::new(
                SnapshotFormat::Full {
                    checkpoint_meta: full_meta,
                },
                bytes.len() as u64,
                timestamp,
            ),
            payload: bytes,
        };
        let envelope_bytes = postcard::to_stdvec(&envelope)
            .map_err(|e| Error::Snapshot(format!("encode full envelope failed: {}", e)))?;

        let mut snapshot_meta = openraft::SnapshotMeta::<RaftTypeConfig>::default();
        snapshot_meta.last_log_id = Some(openraft::LogId::<RaftTypeConfig>::new_term_index(
            self.build_cfg.current_term,
            applied_index,
        ));

        // Phase-2 policy: startup-injected single-node membership baseline.
        let mut voters = BTreeSet::new();
        voters.insert(self.build_cfg.node_id);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            self.build_cfg.node_id,
            openraft::BasicNode::new(&self.build_cfg.node_addr),
        );
        let membership = openraft::Membership::new(vec![voters], nodes)
            .map_err(|e| Error::Snapshot(format!("membership build failed: {}", e)))?;
        snapshot_meta.last_membership = openraft::StoredMembership::new(None, membership);

        snapshot_meta.snapshot_id = format!("full-{}-{}", applied_index, timestamp);

        Ok(Snapshot {
            meta: snapshot_meta,
            snapshot: Cursor::new(envelope_bytes),
        })
    }

    /// Install snapshot through the restore pipeline.
    pub async fn install_snapshot(&mut self, snapshot: Snapshot<RaftTypeConfig>) -> Result<()> {
        let ts = now_secs();
        let restore_dir = PathBuf::from("target/restored").join(format!("snapshot-{}", ts));

        let Snapshot { meta, snapshot } = snapshot;
        let raw = snapshot.into_inner();
        if let Ok(envelope) = postcard::from_bytes::<SnapshotEnvelope>(&raw) {
            match envelope.metadata.format {
                SnapshotFormat::Full { .. } => {
                    let applied_index = meta.last_log_id.as_ref().map(|x| x.index).unwrap_or(0);
                    let term = meta
                        .last_log_id
                        .as_ref()
                        .map(|x| **x.committed_leader_id())
                        .unwrap_or(0);
                    let expected =
                        CheckpointMetadata::new(applied_index, term, 0, restore_dir.clone());

                    SnapshotRestorer::restore(&envelope.payload, restore_dir, &expected).await?;
                    return Ok(());
                }
                SnapshotFormat::Delta { .. } => {
                    return Err(Error::Snapshot(
                        "delta snapshot requires storage-level apply via decoded entries"
                            .to_string(),
                    ));
                }
            }
        }

        // Backward compatibility for old full snapshots written before envelope support.
        let applied_index = meta.last_log_id.as_ref().map(|x| x.index).unwrap_or(0);
        let term = meta
            .last_log_id
            .as_ref()
            .map(|x| **x.committed_leader_id())
            .unwrap_or(0);
        let expected = CheckpointMetadata::new(applied_index, term, 0, restore_dir.clone());

        SnapshotRestorer::restore(&raw, restore_dir, &expected).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CheckpointMetadata as PersistedCheckpointMetadata, SnapshotMetaState};
    use surrealkv::TreeBuilder;
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_snapshot_builder_creation() {
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path("target/test_snapshot_builder".into())
                .build()
                .unwrap(),
        );
        let mut builder = RaftSnapshotBuilder::new(tree);
        let snapshot = builder.build_snapshot(100).await.unwrap();

        assert!(!snapshot.snapshot.get_ref().is_empty());
        assert!(!snapshot.meta.snapshot_id.is_empty());
        assert_eq!(
            snapshot.meta.last_log_id.as_ref().map(|x| x.index),
            Some(100)
        );
    }

    #[tokio::test]
    async fn test_build_and_install_snapshot_pipeline() {
        let base = TempDir::new().unwrap();
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path(base.path().join("tree").into())
                .build()
                .unwrap(),
        );

        let mut builder = RaftSnapshotBuilder::new_with_config(
            tree,
            SnapshotBuildConfig {
                node_id: 1,
                node_addr: "127.0.0.1:50051".to_string(),
                current_term: 7,
            },
        );
        let snapshot = builder.build_snapshot(7).await.unwrap();
        assert!(!snapshot.snapshot.get_ref().is_empty());
        assert_eq!(
            snapshot
                .meta
                .last_log_id
                .as_ref()
                .map(|x| **x.committed_leader_id()),
            Some(7)
        );
        assert!(
            snapshot
                .meta
                .last_membership
                .membership()
                .get_node(&1)
                .is_some()
        );

        builder.install_snapshot(snapshot).await.unwrap();

        // Verify restore root exists after install.
        let restored_root = PathBuf::from("target/restored");
        assert!(fs::metadata(restored_root).await.is_ok());
    }

    #[tokio::test]
    async fn test_decode_snapshot_payload_full_and_delta() {
        let delta_entries = vec![DeltaEntry::new(1, 2, postcard::to_stdvec(&"x").unwrap())];
        let compressed = DeltaSnapshotCodec::encode_entries(&delta_entries).unwrap();

        let delta_envelope = SnapshotEnvelope {
            metadata: DeltaMetadata::new(
                SnapshotFormat::Delta {
                    base_index: 1,
                    entry_count: delta_entries.len() as u64,
                },
                compressed.len() as u64,
                1,
            ),
            payload: compressed,
        };
        let delta_bytes = postcard::to_stdvec(&delta_envelope).unwrap();
        let decoded_delta = decode_snapshot_payload(&delta_bytes).unwrap();
        assert!(matches!(
            decoded_delta,
            Some(DecodedSnapshotPayload::Delta { .. })
        ));

        let full_envelope = SnapshotEnvelope {
            metadata: DeltaMetadata::new(
                SnapshotFormat::Full {
                    checkpoint_meta: CheckpointMetadata::new(1, 1, 0, PathBuf::from("/tmp")),
                },
                3,
                2,
            ),
            payload: vec![1, 2, 3],
        };
        let full_bytes = postcard::to_stdvec(&full_envelope).unwrap();
        let decoded_full = decode_snapshot_payload(&full_bytes).unwrap();
        assert!(matches!(
            decoded_full,
            Some(DecodedSnapshotPayload::Full(_))
        ));
    }

    #[tokio::test]
    async fn test_decode_snapshot_payload_v1_compatibility() {
        let entries = vec![DeltaEntry::new(
            1,
            10,
            postcard::to_stdvec(&"legacy").unwrap(),
        )];
        let compressed = DeltaSnapshotCodec::encode_entries(&entries).unwrap();

        let legacy = LegacySnapshotEnvelopeV1 {
            metadata: LegacyDeltaMetadataV1 {
                format: LegacySnapshotFormatV1::Delta {
                    base_index: 9,
                    entries: entries.clone(),
                },
                size_bytes: compressed.len() as u64,
                created_at: 1,
                file_path: None,
                version: 1,
            },
            payload: compressed,
        };

        let bytes = postcard::to_stdvec(&legacy).unwrap();
        let decoded = decode_snapshot_payload(&bytes).unwrap();
        match decoded {
            Some(DecodedSnapshotPayload::Delta {
                metadata,
                entries: out,
            }) => {
                assert_eq!(out, entries);
                assert_eq!(metadata.version, 2);
                assert!(matches!(
                    metadata.format,
                    SnapshotFormat::Delta {
                        base_index: 9,
                        entry_count: 1
                    }
                ));
            }
            _ => panic!("expected decoded delta payload"),
        }
    }

    #[test]
    fn test_should_build_delta_5mb_threshold() {
        let mut snapshot_state = SnapshotMetaState::new();
        snapshot_state.last_checkpoint =
            Some(PersistedCheckpointMetadata::new(1, 1, 0, now_secs()));

        assert!(should_build_delta(
            &snapshot_state,
            DELTA_ENTRY_THRESHOLD,
            DELTA_MAX_BYTES - 1,
            now_secs(),
        ));

        assert!(!should_build_delta(
            &snapshot_state,
            DELTA_ENTRY_THRESHOLD,
            DELTA_MAX_BYTES + 1,
            now_secs(),
        ));
    }

    #[tokio::test]
    async fn test_decode_delta_entries_from_payload() {
        let entries = vec![DeltaEntry::new(1, 11, postcard::to_stdvec(&"x").unwrap())];
        let compressed = DeltaSnapshotCodec::encode_entries(&entries).unwrap();
        let envelope = SnapshotEnvelope {
            metadata: DeltaMetadata::new(
                SnapshotFormat::Delta {
                    base_index: 10,
                    entry_count: entries.len() as u64,
                },
                compressed.len() as u64,
                1,
            ),
            payload: compressed,
        };

        let bytes = postcard::to_stdvec(&envelope).unwrap();
        let decoded = decode_delta_entries_from_payload(&bytes).unwrap().unwrap();
        assert_eq!(decoded, entries);
    }
}
