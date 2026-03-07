//! SurrealKV-based Raft storage layer - OpenRaft 0.10.0-alpha.15 Compatible Implementation
//!
//! This module provides unified storage backend for:
//! - Raft log entries
//! - State machine application state
//! - Metadata (voting state, applied state, snapshot metadata)
//!
//! The storage adapts to work with both OpenRaft versions through the raft_adapter module.

use crate::error::{Error, Result};
use crate::merge::{
    CheckpointMergeBackend, DeltaMergePolicy, MergeCleanup, MergeCleanupConfig, MergeExecutor,
};
use crate::metrics::{MergeMetrics, SnapshotStage, record_snapshot_strict_error};
use crate::snapshot::{
    DecodedSnapshotPayload, RaftSnapshotBuilder, SnapshotBuildConfig, decode_snapshot_payload,
};
use crate::state::{
    AppliedState, CheckpointMetadata as PersistedCheckpointMetadata, DeltaInfo, MetadataManager,
};
use crate::types::{DeltaEntry, KVRequest, KVResponse, RaftTypeConfig, SnapshotFormat};
use openraft::Snapshot;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use surrealkv::Tree;
use tokio::sync::RwLock;
use tracing::{error, warn};

const ERR_SNAPSHOT_INSTALL_BASE_MISMATCH: &str = "SNAPSHOT_INSTALL_BASE_MISMATCH";
const ERR_SNAPSHOT_INSTALL_INVALID_PAYLOAD: &str = "SNAPSHOT_INSTALL_INVALID_PAYLOAD";
const ERR_SNAPSHOT_INSTALL_DELTA_DECODE: &str = "SNAPSHOT_INSTALL_DELTA_DECODE";
const ERR_SNAPSHOT_INSTALL_DELTA_APPLY_DECODE: &str = "SNAPSHOT_INSTALL_DELTA_APPLY_DECODE";

fn snapshot_error(
    kind: &str,
    stage: SnapshotStage,
    detail: impl Into<String>,
    base_index: u64,
    applied_index: u64,
) -> Error {
    let detail = detail.into();
    record_snapshot_strict_error(kind, stage, base_index, applied_index);
    error!(
        error_key = kind,
        snapshot_stage = stage.as_str(),
        base_index,
        applied_index,
        detail = %detail,
        "strict snapshot error kind={} stage={} base_index={} applied_index={}",
        kind,
        stage.as_str(),
        base_index,
        applied_index,
    );
    Error::Snapshot(format!("{}: {}", kind, detail))
}

/// SurrealStorage - unified storage backend for Raft logs, state machine, and metadata
#[derive(Clone)]
pub struct SurrealStorage {
    tree: Arc<Tree>,
    pub(crate) metadata: Arc<MetadataManager>,
    state_machine: Arc<RwLock<StateMachine>>,
    pub(crate) raft_logs: Arc<RwLock<BTreeMap<u64, LogEntry>>>,
    // Generic OpenRaft log entries for consensus replication.
    pub(crate) raft_entries: Arc<RwLock<BTreeMap<u64, openraft::Entry<RaftTypeConfig>>>>,
    pub(crate) last_purged_log_id: Arc<RwLock<Option<openraft::LogId<RaftTypeConfig>>>>,
    pub(crate) last_membership: Arc<RwLock<openraft::StoredMembership<RaftTypeConfig>>>,
    pub(crate) current_snapshot: Arc<RwLock<Option<Snapshot<RaftTypeConfig>>>>,
    merge_executor: Option<Arc<MergeExecutor>>,
}

/// In-memory state machine that stores application data
#[derive(Debug, Default, Clone)]
pub struct StateMachine {
    data: std::collections::HashMap<String, Vec<u8>>,
}

fn default_stored_membership() -> openraft::StoredMembership<RaftTypeConfig> {
    let mut voters = BTreeSet::new();
    voters.insert(1);
    let membership = openraft::Membership::new_with_defaults(vec![voters], [1]);
    openraft::StoredMembership::new(None, membership)
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
            raft_entries: Arc::new(RwLock::new(BTreeMap::new())),
            last_purged_log_id: Arc::new(RwLock::new(None)),
            last_membership: Arc::new(RwLock::new(default_stored_membership())),
            current_snapshot: Arc::new(RwLock::new(None)),
            merge_executor: None,
        })
    }

    /// Enable automatic background merge with custom policy
    pub fn with_merge_policy(
        mut self,
        policy: DeltaMergePolicy,
        node_id: impl Into<String>,
    ) -> Self {
        let backend = Arc::new(CheckpointMergeBackend::new());
        let executor = MergeExecutor::new(
            self.metadata.clone(),
            policy,
            MergeMetrics::new(node_id),
            MergeCleanup::new(MergeCleanupConfig::default()),
        )
        .with_backend(backend);
        self.merge_executor = Some(Arc::new(executor));
        self
    }

    /// Enable automatic background merge with default policy
    pub fn with_default_merge(self, node_id: impl Into<String>) -> Self {
        self.with_merge_policy(DeltaMergePolicy::default(), node_id)
    }

    /// Recover from incomplete merge task after crash/restart.
    ///
    /// This method inspects persisted MergeProgressState and decides:
    /// - If merge was in_progress: retry the merge task
    /// - If merge completed: no action (idempotent)
    /// - If merge failed with max retries: clear state and allow new merge
    pub async fn recover_merge_state(&self) -> Result<()> {
        let progress = self.metadata.get_merge_progress_state().await;

        if !progress.in_progress {
            // Merge was not in progress or already finished — no action required
            return Ok(());
        }

        warn!(
            trigger = progress.trigger_reason.as_deref().unwrap_or("unknown"),
            attempt = progress.attempt,
            max_retries = progress.max_retries,
            started_at = progress.started_at,
            "detected incomplete merge task from previous session, attempting recovery"
        );

        // If the retry count has already exceeded the configured max, clear the state to
        // allow future merge attempts to start fresh. This protects from perpetual retry loops.
        if progress.attempt >= progress.max_retries {
            error!(
                trigger = progress.trigger_reason.as_deref().unwrap_or("unknown"),
                attempt = progress.attempt,
                max_retries = progress.max_retries,
                last_error = progress.last_error.as_deref().unwrap_or("none"),
                "merge task exhausted retries, clearing state to allow future merge"
            );

            // Clear the persisted failure state so subsequent snapshot builds may trigger a new merge
            use crate::state::MergeProgressState;
            self.metadata
                .save_merge_progress_state(MergeProgressState::new())
                .await?;
            return Ok(());
        }

        // If a merge executor is configured, try to re-spawn the merge task according to policy.
        if let Some(executor) = &self.merge_executor {
            match executor.spawn_if_needed().await {
                Ok(Some(handle)) => {
                    tracing::info!(
                        trigger = handle.trigger.as_str(),
                        "merge recovery task spawned successfully"
                    );
                }
                Ok(None) => {
                    tracing::info!("merge recovery skipped: policy conditions no longer met");
                    // Policy no longer satisfied — clear in-progress flag to avoid repeated attempts
                    use crate::state::MergeProgressState;
                    self.metadata
                        .save_merge_progress_state(MergeProgressState::new())
                        .await?;
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "merge recovery spawn failed, will retry on next snapshot build"
                    );
                    return Err(e);
                }
            }
        } else {
            warn!("merge executor not configured, cannot recover merge task");
        }

        Ok(())
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
    ///
    /// After snapshot creation, automatically triggers background merge if policy is met.
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

                    // Use the full-checkpoint timestamp as the baseline directory locator.
                    let checkpoint_created_at = match &meta.format {
                        SnapshotFormat::Full { checkpoint_meta } => checkpoint_meta.timestamp,
                        SnapshotFormat::Delta { .. } => meta.created_at,
                    };

                    let checkpoint_path = match &meta.format {
                        SnapshotFormat::Full { checkpoint_meta } => checkpoint_meta
                            .checkpoint_dir
                            .join(checkpoint_meta.dir_name())
                            .to_string_lossy()
                            .to_string(),
                        SnapshotFormat::Delta { .. } => {
                            format!("target/checkpoints/checkpoint_{}", checkpoint_created_at)
                        }
                    };

                    snapshot_state.last_checkpoint = Some(
                        PersistedCheckpointMetadata::new(index, term, 0, checkpoint_created_at)
                            .with_checkpoint_path(checkpoint_path),
                    );
                    snapshot_state.delta_chain.clear();
                    snapshot_state.total_delta_bytes = 0;
                }
                DecodedSnapshotPayload::Delta { metadata, entries } => {
                    let start_index = entries.first().map(|x| x.index).unwrap_or(0);
                    let end_index = entries.last().map(|x| x.index).unwrap_or(start_index);
                    let mut info = DeltaInfo::new(
                        start_index,
                        end_index,
                        metadata.size_bytes,
                        metadata.created_at,
                    );
                    if let Some(path) = metadata.file_path {
                        info = info.with_file_path(path);
                    }
                    snapshot_state.delta_chain.push(info);
                    snapshot_state.total_delta_bytes = snapshot_state
                        .total_delta_bytes
                        .saturating_add(metadata.size_bytes);
                }
            }

            self.metadata.save_snapshot_state(snapshot_state).await?;
        }

        // Phase 4: Trigger background merge if policy is met
        if let Some(executor) = &self.merge_executor {
            if let Ok(Some(handle)) = executor.spawn_if_needed().await {
                tracing::info!(
                    trigger = handle.trigger.as_str(),
                    "background merge task spawned after snapshot creation"
                );
                // Return snapshot immediately; merge continues in background (non-blocking).
            }
        }

        Ok(snapshot)
    }

    /// Install full or delta snapshot depending on payload format.
    pub async fn install_snapshot_auto(&self, snapshot: Snapshot<RaftTypeConfig>) -> Result<()> {
        let decoded = decode_snapshot_payload(snapshot.snapshot.get_ref()).map_err(|e| {
            snapshot_error(
                ERR_SNAPSHOT_INSTALL_DELTA_DECODE,
                SnapshotStage::DecodePayload,
                e.to_string(),
                0,
                0,
            )
        })?;

        if let Some(decoded) = decoded {
            return match decoded {
                DecodedSnapshotPayload::Delta { metadata, entries } => {
                    let base_index = match metadata.format {
                        SnapshotFormat::Delta { base_index, .. } => base_index,
                        SnapshotFormat::Full { .. } => 0,
                    };

                    let applied = self.metadata.get_applied_state().await;
                    if applied.last_applied_index != base_index {
                        warn!(
                            error_key = ERR_SNAPSHOT_INSTALL_BASE_MISMATCH,
                            snapshot_stage = SnapshotStage::ValidateBase.as_str(),
                            expected_base_index = base_index,
                            applied_index = applied.last_applied_index,
                            "delta base index mismatch during install"
                        );
                        return Err(snapshot_error(
                            ERR_SNAPSHOT_INSTALL_BASE_MISMATCH,
                            SnapshotStage::ValidateBase,
                            format!(
                                "expected={}, got={}",
                                base_index, applied.last_applied_index
                            ),
                            base_index,
                            applied.last_applied_index,
                        ));
                    }

                    self.apply_delta_entries(&entries).await?;
                    Ok(())
                }
                DecodedSnapshotPayload::Full(_) => {
                    let mut builder = RaftSnapshotBuilder::new(self.tree.clone());
                    builder.install_snapshot(snapshot).await
                }
            };
        }

        warn!(
            error_key = ERR_SNAPSHOT_INSTALL_INVALID_PAYLOAD,
            snapshot_stage = SnapshotStage::RejectPayload.as_str(),
            base_index = 0,
            applied_index = 0,
            "snapshot payload rejected because it is unknown or invalid"
        );
        Err(snapshot_error(
            ERR_SNAPSHOT_INSTALL_INVALID_PAYLOAD,
            SnapshotStage::RejectPayload,
            "unknown or invalid snapshot payload",
            0,
            0,
        ))
    }

    /// Re-apply delta entries to state machine.
    pub async fn apply_delta_entries(&self, entries: &[DeltaEntry]) -> Result<()> {
        for e in entries {
            let req: KVRequest = postcard::from_bytes(&e.payload).map_err(|err| {
                error!(
                    error_key = ERR_SNAPSHOT_INSTALL_DELTA_APPLY_DECODE,
                    snapshot_stage = SnapshotStage::ApplyDelta.as_str(),
                    base_index = 0,
                    applied_index = 0,
                    entry_index = e.index,
                    cause = %err,
                    "failed to decode delta entry payload"
                );
                snapshot_error(
                    ERR_SNAPSHOT_INSTALL_DELTA_APPLY_DECODE,
                    SnapshotStage::ApplyDelta,
                    format!("index={}, cause={}", e.index, err),
                    0,
                    0,
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

/// Snapshot builder compatibility shim kept for API stability.
///
/// TODO(breaking-change): remove this legacy type in a dedicated cleanup release.
#[allow(dead_code)]
pub struct SnapshotBuilderImpl {
    tree: Arc<Tree>,
}

impl SnapshotBuilderImpl {
    pub fn new(tree: Arc<Tree>) -> Self {
        SnapshotBuilderImpl { tree }
    }
}

// OpenRaft trait implementations are in storage_raft_impl.rs module
// This keeps storage.rs focused on core storage logic

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
