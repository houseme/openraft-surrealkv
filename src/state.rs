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
}

impl DeltaInfo {
    pub fn new(start: u64, end: u64, size: u64, timestamp: u64) -> Self {
        DeltaInfo {
            start_index: start,
            end_index: end,
            size_bytes: size,
            created_at: timestamp,
        }
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
}

impl MetadataManager {
    /// Create a new metadata manager with the given SurrealKV tree
    pub fn new(tree: Arc<Tree>) -> Self {
        MetadataManager {
            tree,
            voting_state: Arc::new(RwLock::new(VotingState::new())),
            applied_state: Arc::new(RwLock::new(AppliedState::new())),
            snapshot_state: Arc::new(RwLock::new(SnapshotMetaState::new())),
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
        let info = DeltaInfo::new(100, 150, 5000, 1234567890);
        assert_eq!(info.start_index, 100);
        assert_eq!(info.end_index, 150);
        assert_eq!(info.size_bytes, 5000);
        assert_eq!(info.created_at, 1234567890);
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
