//! SurrealKV-based Raft storage layer - OpenRaft 0.10.0-alpha.15 Compatible Implementation
//!
//! This module provides unified storage backend for:
//! - Raft log entries
//! - State machine application state
//! - Metadata (voting state, applied state, snapshot metadata)
//!
//! The storage adapts to work with both OpenRaft versions through the raft_adapter module.

use crate::error::{Error, Result};
use crate::snapshot::{
    decode_snapshot_payload, DecodedSnapshotPayload, RaftSnapshotBuilder, SnapshotBuildConfig,
};
use crate::state::{
    AppliedState, CheckpointMetadata as PersistedCheckpointMetadata, DeltaInfo, MetadataManager,
};
use crate::types::{DeltaEntry, KVRequest, KVResponse, RaftTypeConfig, SnapshotFormat};
use openraft::Snapshot;
use std::collections::BTreeMap;
use std::sync::Arc;
use surrealkv::Tree;
use tokio::sync::RwLock;

const ERR_SNAPSHOT_INSTALL_BASE_MISMATCH: &str = "SNAPSHOT_INSTALL_BASE_MISMATCH";
const ERR_SNAPSHOT_INSTALL_INVALID_PAYLOAD: &str = "SNAPSHOT_INSTALL_INVALID_PAYLOAD";
const ERR_SNAPSHOT_INSTALL_DELTA_DECODE: &str = "SNAPSHOT_INSTALL_DELTA_DECODE";
const ERR_SNAPSHOT_INSTALL_DELTA_APPLY_DECODE: &str = "SNAPSHOT_INSTALL_DELTA_APPLY_DECODE";

fn snapshot_error(kind: &str, detail: impl Into<String>) -> Error {
    Error::Snapshot(format!("{}: {}", kind, detail.into()))
}

/// SurrealStorage - unified storage backend for Raft logs, state machine, and metadata
pub struct SurrealStorage {
    tree: Arc<Tree>,
    metadata: Arc<MetadataManager>,
    state_machine: Arc<RwLock<StateMachine>>,
    raft_logs: Arc<RwLock<BTreeMap<u64, LogEntry>>>,
}

/// In-memory state machine that stores application data
#[derive(Debug, Default, Clone)]
pub struct StateMachine {
    data: std::collections::HashMap<String, Vec<u8>>,
}

impl SurrealStorage {
    /// Create a new SurrealStorage with the given tree
    pub async fn new(tree: Arc<Tree>) -> Result<Self> {
        let metadata = Arc::new(MetadataManager::new(tree.clone()));
        metadata.load().await?;

        Ok(SurrealStorage {
            tree,
            metadata,
            state_machine: Arc::new(RwLock::new(StateMachine::default())),
            raft_logs: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    pub fn metadata(&self) -> Arc<MetadataManager> {
        self.metadata.clone()
    }

    pub fn tree(&self) -> Arc<Tree> {
        self.tree.clone()
    }

    pub async fn read(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let sm = self.state_machine.read().await;
        Ok(sm.data.get(key).cloned())
    }

    pub async fn apply_request(&self, req: &KVRequest) -> Result<KVResponse> {
        let mut sm = self.state_machine.write().await;
        match req {
            KVRequest::Set { key, value } => {
                sm.data.insert(key.clone(), value.clone());
                Ok(KVResponse::Ok)
            }
            KVRequest::Delete { key } => {
                sm.data.remove(key);
                Ok(KVResponse::Ok)
            }
        }
    }

    /// Append one applied log entry for snapshot delta generation.
    pub async fn append_log_entry(&self, term: u64, index: u64, req: &KVRequest) -> Result<()> {
        let payload = postcard::to_stdvec(req)?;
        let entry = LogEntry::new(term, index, payload);
        self.raft_logs.write().await.insert(index, entry);

        self.metadata
            .save_applied_state(AppliedState {
                last_applied_index: index,
                last_applied_term: term,
            })
            .await?;
        Ok(())
    }

    /// Return applied entries in [start_index, end_index].
    pub async fn try_get_log_entries(
        &self,
        start_index: u64,
        end_index: u64,
    ) -> Result<Vec<LogEntry>> {
        let logs = self.raft_logs.read().await;
        let mut out = Vec::new();
        for (_, entry) in logs.range(start_index..=end_index) {
            out.push(entry.clone());
        }
        Ok(out)
    }

    /// Build snapshot and select full/delta automatically using metadata policy.
    pub async fn build_snapshot_auto(&self, current_term: u64) -> Result<Snapshot<RaftTypeConfig>> {
        let applied = self.metadata.get_applied_state().await;
        let mut snapshot_state = self.metadata.get_snapshot_state().await;

        let start = snapshot_state
            .last_checkpoint
            .as_ref()
            .map(|c| c.checkpoint_index.saturating_add(1))
            .unwrap_or(1);

        let logs = if applied.last_applied_index >= start {
            self.try_get_log_entries(start, applied.last_applied_index)
                .await?
        } else {
            Vec::new()
        };

        let delta_entries = logs
            .into_iter()
            .map(|l| DeltaEntry::new(l.term, l.index, l.payload))
            .collect();

        let mut builder = RaftSnapshotBuilder::new_with_config(
            self.tree.clone(),
            SnapshotBuildConfig {
                node_id: 1,
                node_addr: "127.0.0.1:50051".to_string(),
                current_term,
            },
        );

        let snapshot = builder
            .build_snapshot_with_delta(applied.last_applied_index, &snapshot_state, delta_entries)
            .await?;

        if let Some(decoded) = decode_snapshot_payload(snapshot.snapshot.get_ref())? {
            match decoded {
                DecodedSnapshotPayload::Full(meta) => {
                    let (index, term) = snapshot
                        .meta
                        .last_log_id
                        .as_ref()
                        .map(|x| (x.index, **x.committed_leader_id()))
                        .unwrap_or((0, 0));

                    snapshot_state.last_checkpoint = Some(PersistedCheckpointMetadata::new(
                        index,
                        term,
                        0,
                        meta.created_at,
                    ));
                    snapshot_state.delta_chain.clear();
                    snapshot_state.total_delta_bytes = 0;
                }
                DecodedSnapshotPayload::Delta { metadata, entries } => {
                    let start_index = entries.first().map(|x| x.index).unwrap_or(0);
                    let end_index = entries.last().map(|x| x.index).unwrap_or(start_index);
                    snapshot_state.delta_chain.push(DeltaInfo::new(
                        start_index,
                        end_index,
                        metadata.size_bytes,
                        metadata.created_at,
                    ));
                    snapshot_state.total_delta_bytes = snapshot_state
                        .total_delta_bytes
                        .saturating_add(metadata.size_bytes);
                }
            }

            self.metadata.save_snapshot_state(snapshot_state).await?;
        }

        Ok(snapshot)
    }

    /// Install full or delta snapshot depending on payload format.
    pub async fn install_snapshot_auto(&self, snapshot: Snapshot<RaftTypeConfig>) -> Result<()> {
        let decoded = decode_snapshot_payload(snapshot.snapshot.get_ref())
            .map_err(|e| snapshot_error(ERR_SNAPSHOT_INSTALL_DELTA_DECODE, e.to_string()))?;

        if let Some(decoded) = decoded {
            match decoded {
                DecodedSnapshotPayload::Delta { metadata, entries } => {
                    let base_index = match metadata.format {
                        SnapshotFormat::Delta { base_index, .. } => base_index,
                        SnapshotFormat::Full { .. } => 0,
                    };

                    let applied = self.metadata.get_applied_state().await;
                    if applied.last_applied_index != base_index {
                        return Err(snapshot_error(
                            ERR_SNAPSHOT_INSTALL_BASE_MISMATCH,
                            format!(
                                "expected={}, got={}",
                                base_index, applied.last_applied_index
                            ),
                        ));
                    }

                    self.apply_delta_entries(&entries).await?;
                    return Ok(());
                }
                DecodedSnapshotPayload::Full(_) => {
                    let mut builder = RaftSnapshotBuilder::new(self.tree.clone());
                    return builder.install_snapshot(snapshot).await;
                }
            }
        }

        Err(snapshot_error(
            ERR_SNAPSHOT_INSTALL_INVALID_PAYLOAD,
            "unknown or invalid snapshot payload",
        ))
    }

    /// Re-apply delta entries to state machine.
    pub async fn apply_delta_entries(&self, entries: &[DeltaEntry]) -> Result<()> {
        for e in entries {
            let req: KVRequest = postcard::from_bytes(&e.payload).map_err(|err| {
                snapshot_error(
                    ERR_SNAPSHOT_INSTALL_DELTA_APPLY_DECODE,
                    format!("index={}, cause={}", e.index, err),
                )
            })?;
            self.apply_request(&req).await?;
            self.append_log_entry(e.term, e.index, &req).await?;
        }
        Ok(())
    }
}

/// Raft log entry structure
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub term: u64,
    pub index: u64,
    pub payload: Vec<u8>,
}

impl LogEntry {
    pub fn new(term: u64, index: u64, payload: Vec<u8>) -> Self {
        LogEntry {
            term,
            index,
            payload,
        }
    }

    pub fn storage_key(term: u64, index: u64) -> Vec<u8> {
        format!("raft_log:{}:{}", term, index).into_bytes()
    }
}

/// Snapshot builder implementation - placeholder for Phase 1
pub struct SnapshotBuilderImpl {
    tree: Arc<Tree>,
}

impl SnapshotBuilderImpl {
    pub fn new(tree: Arc<Tree>) -> Self {
        SnapshotBuilderImpl { tree }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::DeltaSnapshotCodec;
    use crate::types::DeltaMetadata;
    use serde::Serialize;
    use std::io::Cursor;

    #[derive(Serialize)]
    struct TestEnvelope {
        metadata: DeltaMetadata,
        payload: Vec<u8>,
    }

    #[test]
    fn test_log_entry_storage_key() {
        let key = LogEntry::storage_key(5, 100);
        assert_eq!(key, b"raft_log:5:100".to_vec());
    }

    #[tokio::test]
    async fn test_apply_set_request() {
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_apply_set".into())
                .build()
                .unwrap(),
        );
        let storage = SurrealStorage::new(tree).await.unwrap();

        let req = KVRequest::Set {
            key: "test".to_string(),
            value: vec![1, 2, 3],
        };
        let resp = storage.apply_request(&req).await.unwrap();
        assert_eq!(resp, KVResponse::Ok);

        let value = storage.read("test").await.unwrap();
        assert_eq!(value, Some(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn test_apply_delete_request() {
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_apply_delete".into())
                .build()
                .unwrap(),
        );
        let storage = SurrealStorage::new(tree).await.unwrap();

        let req = KVRequest::Set {
            key: "test".to_string(),
            value: vec![1, 2, 3],
        };
        storage.apply_request(&req).await.unwrap();

        let req = KVRequest::Delete {
            key: "test".to_string(),
        };
        storage.apply_request(&req).await.unwrap();

        let value = storage.read("test").await.unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = LogEntry::new(5, 100, vec![1, 2, 3]);
        let bytes = postcard::to_stdvec(&entry).unwrap();
        let deserialized: LogEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(entry, deserialized);
    }

    #[tokio::test]
    async fn test_try_get_log_entries_range() {
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_log_range".into())
                .build()
                .unwrap(),
        );
        let storage = SurrealStorage::new(tree).await.unwrap();

        for i in 1..=5 {
            let req = KVRequest::Set {
                key: format!("k{}", i),
                value: vec![i as u8],
            };
            storage.append_log_entry(1, i, &req).await.unwrap();
        }

        let entries = storage.try_get_log_entries(2, 4).await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].index, 2);
        assert_eq!(entries[2].index, 4);
    }

    #[tokio::test]
    async fn test_install_snapshot_auto_rejects_base_index_mismatch() {
        let dst_tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_delta_base_dst".into())
                .build()
                .unwrap(),
        );
        let dst = SurrealStorage::new(dst_tree).await.unwrap();

        // Force destination applied index away from delta base index.
        let req = KVRequest::Set {
            key: "k".to_string(),
            value: vec![1],
        };
        dst.append_log_entry(1, 1, &req).await.unwrap();

        // Build a synthetic delta snapshot with base_index=0.
        let delta_entries = vec![DeltaEntry::new(
            1,
            1,
            postcard::to_stdvec(&KVRequest::Set {
                key: "x".to_string(),
                value: vec![9],
            })
            .unwrap(),
        )];
        let compressed = DeltaSnapshotCodec::encode_entries(&delta_entries).unwrap();
        let envelope = TestEnvelope {
            metadata: DeltaMetadata::new(
                SnapshotFormat::Delta {
                    base_index: 0,
                    entry_count: delta_entries.len() as u64,
                },
                compressed.len() as u64,
                1,
            ),
            payload: compressed,
        };
        let bytes = postcard::to_stdvec(&envelope).unwrap();

        let delta_snapshot = Snapshot {
            meta: openraft::SnapshotMeta::<RaftTypeConfig>::default(),
            snapshot: Cursor::new(bytes),
        };

        let err = dst.install_snapshot_auto(delta_snapshot).await.unwrap_err();
        assert!(err.to_string().contains(ERR_SNAPSHOT_INSTALL_BASE_MISMATCH));
    }

    #[tokio::test]
    async fn test_install_snapshot_auto_rejects_unknown_payload() {
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_invalid_snapshot_payload".into())
                .build()
                .unwrap(),
        );
        let storage = SurrealStorage::new(tree).await.unwrap();

        let snapshot = Snapshot {
            meta: openraft::SnapshotMeta::<RaftTypeConfig>::default(),
            snapshot: Cursor::new(vec![1, 2, 3, 4, 5]),
        };

        let err = storage.install_snapshot_auto(snapshot).await.unwrap_err();
        assert!(
            err.to_string()
                .contains(ERR_SNAPSHOT_INSTALL_INVALID_PAYLOAD)
        );
    }

    #[tokio::test]
    async fn test_install_snapshot_auto_rejects_corrupted_delta_payload() {
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_corrupted_delta_payload".into())
                .build()
                .unwrap(),
        );
        let storage = SurrealStorage::new(tree).await.unwrap();

        // Shape is a valid envelope, but delta payload is invalid zstd/postcard bytes.
        let envelope = TestEnvelope {
            metadata: DeltaMetadata::new(
                SnapshotFormat::Delta {
                    base_index: 0,
                    entry_count: 1,
                },
                5,
                1,
            ),
            payload: vec![1, 2, 3, 4, 5],
        };
        let bytes = postcard::to_stdvec(&envelope).unwrap();

        let snapshot = Snapshot {
            meta: openraft::SnapshotMeta::<RaftTypeConfig>::default(),
            snapshot: Cursor::new(bytes),
        };

        let err = storage.install_snapshot_auto(snapshot).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(ERR_SNAPSHOT_INSTALL_DELTA_DECODE));
    }
}
