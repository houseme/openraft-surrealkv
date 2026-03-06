//! Persistent metadata state management for Raft.
//!
//! This module manages persistent Raft state including:
//! - Voting state (term, voted_for)
//! - Applied log state (last applied index/term)
//! - Snapshot metadata (checkpoint state, delta chain info)
//!
//! All state is stored in SurrealKV with immediate persistence for voting.

use crate::error::Result;
use crate::types::NodeId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use surrealkv::Tree;
use tokio::sync::RwLock;

/// Voting state - persisted immediately for Raft safety
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VotingState {
    pub current_term: u64,
    pub voted_for: Option<NodeId>,
}

impl VotingState {
    pub fn new() -> Self {
        VotingState {
            current_term: 0,
            voted_for: None,
        }
    }
}

impl Default for VotingState {
    fn default() -> Self {
        Self::new()
    }
}

/// Applied log state - tracks what has been applied to state machine
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedState {
    pub last_applied_index: u64,
    pub last_applied_term: u64,
}

impl AppliedState {
    pub fn new() -> Self {
        AppliedState {
            last_applied_index: 0,
            last_applied_term: 0,
        }
    }
}

impl Default for AppliedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Checkpoint metadata - information about the last full snapshot
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointMetadata {
    /// Index of the log entry at checkpoint time
    pub checkpoint_index: u64,
    /// Term of the log entry at checkpoint time
    pub checkpoint_term: u64,
    /// SurrealKV sequence number for consistency verification
    pub sequence_number: u64,
    /// Timestamp of checkpoint creation
    pub created_at: u64,
}

impl CheckpointMetadata {
    pub fn new(index: u64, term: u64, seq: u64, timestamp: u64) -> Self {
        CheckpointMetadata {
            checkpoint_index: index,
            checkpoint_term: term,
            sequence_number: seq,
            created_at: timestamp,
        }
    }
}

/// Delta snapshot information - tracks incremental snapshots
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeltaInfo {
    /// Index of the first entry in this delta
    pub start_index: u64,
    /// Index of the last entry in this delta
    pub end_index: u64,
    /// Size of compressed delta in bytes
    pub size_bytes: u64,
    /// Timestamp of delta creation
    pub created_at: u64,
    /// Optional persisted file path for cleanup.
    pub file_path: Option<String>,
}

impl DeltaInfo {
    pub fn new(start: u64, end: u64, size: u64, timestamp: u64) -> Self {
        DeltaInfo {
            start_index: start,
            end_index: end,
            size_bytes: size,
            created_at: timestamp,
            file_path: None,
        }
    }

    pub fn with_file_path(mut self, file_path: String) -> Self {
        self.file_path = Some(file_path);
        self
    }
}

/// Merge progress state persisted to avoid duplicate/re-entrant execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergeProgressState {
    pub in_progress: bool,
    pub trigger_reason: Option<String>,
    pub attempt: u8,
    pub max_retries: u8,
    pub started_at: u64,
    pub updated_at: u64,
    pub last_error: Option<String>,
}

impl MergeProgressState {
    pub fn new() -> Self {
        Self {
            in_progress: false,
            trigger_reason: None,
            attempt: 0,
            max_retries: 0,
            started_at: 0,
            updated_at: 0,
            last_error: None,
        }
    }

    pub fn started(trigger_reason: String, attempt: u8, max_retries: u8, now: u64) -> Self {
        Self {
            in_progress: true,
            trigger_reason: Some(trigger_reason),
            attempt,
            max_retries,
            started_at: now,
            updated_at: now,
            last_error: None,
        }
    }

    pub fn succeeded(
        trigger_reason: String,
        attempt: u8,
        max_retries: u8,
        started_at: u64,
    ) -> Self {
        Self {
            in_progress: false,
            trigger_reason: Some(trigger_reason),
            attempt,
            max_retries,
            started_at,
            updated_at: started_at,
            last_error: None,
        }
    }

    pub fn failed(
        trigger_reason: String,
        attempt: u8,
        max_retries: u8,
        started_at: u64,
        last_error: String,
    ) -> Self {
        Self {
            in_progress: false,
            trigger_reason: Some(trigger_reason),
            attempt,
            max_retries,
            started_at,
            updated_at: started_at,
            last_error: Some(last_error),
        }
    }
}

impl Default for MergeProgressState {
    fn default() -> Self {
        Self::new()
    }
}

/// Complete snapshot state - combines checkpoint and delta chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetaState {
    pub last_checkpoint: Option<CheckpointMetadata>,
    pub delta_chain: Vec<DeltaInfo>,
    pub total_delta_bytes: u64,
}

impl SnapshotMetaState {
    pub fn new() -> Self {
        SnapshotMetaState {
            last_checkpoint: None,
            delta_chain: vec![],
            total_delta_bytes: 0,
        }
    }

    /// Check if merge is needed based on delta chain
    pub fn should_merge(&self) -> bool {
        const MAX_DELTA_CHAIN: usize = 5;
        const MAX_DELTA_BYTES: u64 = 300 * 1024 * 1024; // 300MB

        self.delta_chain.len() >= MAX_DELTA_CHAIN || self.total_delta_bytes >= MAX_DELTA_BYTES
    }
}

impl Default for SnapshotMetaState {
    fn default() -> Self {
        Self::new()
    }
}

/// Metadata state manager - wraps all persistent state with thread-safe access
pub struct MetadataManager {
    tree: Arc<Tree>,
    voting_state: Arc<RwLock<VotingState>>,
    applied_state: Arc<RwLock<AppliedState>>,
    snapshot_state: Arc<RwLock<SnapshotMetaState>>,
    merge_progress_state: Arc<RwLock<MergeProgressState>>,
}

impl MetadataManager {
    /// Create a new metadata manager with the given SurrealKV tree
    pub fn new(tree: Arc<Tree>) -> Self {
        MetadataManager {
            tree,
            voting_state: Arc::new(RwLock::new(VotingState::new())),
            applied_state: Arc::new(RwLock::new(AppliedState::new())),
            snapshot_state: Arc::new(RwLock::new(SnapshotMetaState::new())),
            merge_progress_state: Arc::new(RwLock::new(MergeProgressState::new())),
        }
    }

    /// Load all metadata from persistent storage
    pub async fn load(&self) -> Result<()> {
        // Create a read transaction
        let txn = self.tree.begin()?;

        // Load voting state
        if let Some(bytes) = txn.get(b"raft_vote")? {
            let state: VotingState = postcard::from_bytes(&bytes)?;
            *self.voting_state.write().await = state;
        }

        // Load applied state
        if let Some(bytes) = txn.get(b"raft_applied")? {
            let state: AppliedState = postcard::from_bytes(&bytes)?;
            *self.applied_state.write().await = state;
        }

        // Load snapshot state
        if let Some(bytes) = txn.get(b"raft_snapshot_meta")? {
            let state: SnapshotMetaState = postcard::from_bytes(&bytes)?;
            *self.snapshot_state.write().await = state;
        }

        // Load merge progress state
        if let Some(bytes) = txn.get(b"raft_merge_progress")? {
            let state: MergeProgressState = postcard::from_bytes(&bytes)?;
            *self.merge_progress_state.write().await = state;
        }

        Ok(())
    }

    /// Save voting state with immediate persistence
    pub async fn save_voting_state(&self, state: VotingState) -> Result<()> {
        let bytes = postcard::to_stdvec(&state)?;
        let mut txn = self.tree.begin()?;
        txn.set(b"raft_vote", &bytes)?;
        txn.commit().await?;
        *self.voting_state.write().await = state;
        Ok(())
    }

    /// Get current voting state
    pub async fn get_voting_state(&self) -> VotingState {
        self.voting_state.read().await.clone()
    }

    /// Save applied state
    pub async fn save_applied_state(&self, state: AppliedState) -> Result<()> {
        let bytes = postcard::to_stdvec(&state)?;
        let mut txn = self.tree.begin()?;
        txn.set(b"raft_applied", &bytes)?;
        txn.commit().await?;
        *self.applied_state.write().await = state;
        Ok(())
    }

    /// Get current applied state
    pub async fn get_applied_state(&self) -> AppliedState {
        self.applied_state.read().await.clone()
    }

    /// Save snapshot metadata state
    pub async fn save_snapshot_state(&self, state: SnapshotMetaState) -> Result<()> {
        let bytes = postcard::to_stdvec(&state)?;
        let mut txn = self.tree.begin()?;
        txn.set(b"raft_snapshot_meta", &bytes)?;
        txn.commit().await?;
        *self.snapshot_state.write().await = state;
        Ok(())
    }

    /// Get current snapshot state
    pub async fn get_snapshot_state(&self) -> SnapshotMetaState {
        self.snapshot_state.read().await.clone()
    }

    /// Save merge progress state.
    pub async fn save_merge_progress_state(&self, state: MergeProgressState) -> Result<()> {
        let bytes = postcard::to_stdvec(&state)?;
        let mut txn = self.tree.begin()?;
        txn.set(b"raft_merge_progress", &bytes)?;
        txn.commit().await?;
        *self.merge_progress_state.write().await = state;
        Ok(())
    }

    /// Get current merge progress state.
    pub async fn get_merge_progress_state(&self) -> MergeProgressState {
        self.merge_progress_state.read().await.clone()
    }

    /// Atomically save snapshot and merge progress states in one transaction.
    pub async fn save_snapshot_and_merge_state(
        &self,
        snapshot_state: SnapshotMetaState,
        merge_state: MergeProgressState,
    ) -> Result<()> {
        let snapshot_bytes = postcard::to_stdvec(&snapshot_state)?;
        let merge_bytes = postcard::to_stdvec(&merge_state)?;

        let mut txn = self.tree.begin()?;
        txn.set(b"raft_snapshot_meta", &snapshot_bytes)?;
        txn.set(b"raft_merge_progress", &merge_bytes)?;
        txn.commit().await?;

        *self.snapshot_state.write().await = snapshot_state;
        *self.merge_progress_state.write().await = merge_state;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voting_state_new() {
        let state = VotingState::new();
        assert_eq!(state.current_term, 0);
        assert_eq!(state.voted_for, None);
    }

    #[test]
    fn test_applied_state_new() {
        let state = AppliedState::new();
        assert_eq!(state.last_applied_index, 0);
        assert_eq!(state.last_applied_term, 0);
    }

    #[test]
    fn test_checkpoint_metadata_new() {
        let meta = CheckpointMetadata::new(100, 5, 12345, 1234567890);
        assert_eq!(meta.checkpoint_index, 100);
        assert_eq!(meta.checkpoint_term, 5);
        assert_eq!(meta.sequence_number, 12345);
        assert_eq!(meta.created_at, 1234567890);
    }

    #[test]
    fn test_delta_info_new() {
        let info =
            DeltaInfo::new(100, 150, 5000, 1234567890).with_file_path("/tmp/delta.zst".to_string());
        assert_eq!(info.start_index, 100);
        assert_eq!(info.end_index, 150);
        assert_eq!(info.size_bytes, 5000);
        assert_eq!(info.created_at, 1234567890);
        assert_eq!(info.file_path.as_deref(), Some("/tmp/delta.zst"));
    }

    #[test]
    fn test_merge_progress_state_new() {
        let state = MergeProgressState::new();
        assert!(!state.in_progress);
        assert_eq!(state.attempt, 0);
        assert_eq!(state.max_retries, 0);
        assert_eq!(state.last_error, None);
    }

    #[test]
    fn test_snapshot_meta_state_should_merge_by_chain() {
        let mut state = SnapshotMetaState::new();
        for i in 0..5 {
            state
                .delta_chain
                .push(DeltaInfo::new(i * 100, (i + 1) * 100, 1000, 0));
        }
        assert!(state.should_merge());
    }

    #[test]
    fn test_snapshot_meta_state_should_merge_by_size() {
        let mut state = SnapshotMetaState::new();
        state.delta_chain.push(DeltaInfo::new(0, 100, 1000, 0));
        state.total_delta_bytes = 300 * 1024 * 1024;
        assert!(state.should_merge());
    }

    #[test]
    fn test_snapshot_meta_state_no_merge() {
        let state = SnapshotMetaState::new();
        assert!(!state.should_merge());
    }

    #[test]
    fn test_voting_state_serialization() {
        let state = VotingState {
            current_term: 5,
            voted_for: Some(42),
        };
        let bytes = postcard::to_stdvec(&state).unwrap();
        let deserialized: VotingState = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(state, deserialized);
    }
}
