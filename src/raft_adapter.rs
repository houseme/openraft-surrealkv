//! OpenRaft version abstraction layer
//!
//! This module provides trait-based abstractions to handle differences between
//! OpenRaft 0.9.21 and 0.10.0-alpha.15, enabling seamless version switching
//! via feature flags.
//!
//! # Design Principles
//! 1. Version-agnostic interface for Phase 0-5 modules
//! 2. Feature flags determine which OpenRaft version is active
//! 3. Minimal overhead - adapters are thin wrappers around underlying traits
//! 4. Type-safe conversion at module boundaries

use crate::error::Result;
use crate::types::SnapshotData;
use std::io;

/// Version-agnostic snapshot metadata
#[derive(Debug, Clone)]
pub struct SnapshotMetadata {
    pub index: u64,
    pub term: u64,
    pub members: Vec<(u64, openraft::BasicNode)>,
}

/// Adapter for RaftLogStorage trait across OpenRaft versions
///
/// This trait abstracts the storage layer to work with both:
/// - openraft 0.10.0-alpha.15 (default)
/// - openraft-legacy 0.10.0-alpha.15 (with feature flag)
#[async_trait::async_trait]
pub trait RaftLogStorageAdapter: Send + Sync {
    /// Append a list of entries to the log
    async fn append(&self, entries: Vec<Vec<u8>>) -> Result<()>;

    /// Delete entries in a range [start, end)
    async fn truncate(&self, from: u64) -> Result<()>;

    /// Purge entries up to this index
    async fn purge(&self, upto: u64) -> Result<()>;
}

/// Adapter for RaftStateMachine trait across OpenRaft versions
#[async_trait::async_trait]
pub trait RaftStateMachineAdapter: Send + Sync {
    /// Apply an entry to the state machine
    async fn apply(&self, entry: Vec<u8>) -> Result<Vec<u8>>;

    /// Get a snapshot of the state machine
    async fn snapshot(&self) -> Result<SnapshotData>;

    /// Install a snapshot
    async fn install_snapshot(&self, meta: SnapshotMetadata, snapshot: SnapshotData) -> Result<()>;
}

/// Configuration common to both OpenRaft versions
#[derive(Debug, Clone)]
pub struct RaftConfig {
    /// Election timeout in milliseconds
    pub election_timeout_ms: u64,
    /// Heartbeat interval in milliseconds
    pub heartbeat_interval_ms: u64,
}

impl Default for RaftConfig {
    fn default() -> Self {
        RaftConfig {
            election_timeout_ms: 150,
            heartbeat_interval_ms: 50,
        }
    }
}

/// Helper to convert between version-specific error types
pub fn convert_openraft_error<E: std::fmt::Display>(err: E) -> crate::error::RaftError {
    crate::error::RaftError::RaftProto(err.to_string())
}

/// Helper to convert IO errors
pub fn convert_io_error(err: io::Error) -> crate::error::RaftError {
    crate::error::RaftError::Io(err)
}

/// Helper to convert serialization errors
pub fn convert_serde_error<E: std::fmt::Display>(err: E) -> crate::error::RaftError {
    crate::error::RaftError::Serialization(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_raft_config() {
        let config = RaftConfig::default();
        assert_eq!(config.election_timeout_ms, 150);
        assert_eq!(config.heartbeat_interval_ms, 50);
    }

    #[test]
    fn test_custom_raft_config() {
        let config = RaftConfig {
            election_timeout_ms: 300,
            heartbeat_interval_ms: 100,
        };
        assert_eq!(config.election_timeout_ms, 300);
        assert_eq!(config.heartbeat_interval_ms, 100);
    }
}
