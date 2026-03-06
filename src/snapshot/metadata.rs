//! Checkpoint metadata for Phase 2 Checkpoint implementation
//!
//! This module defines the metadata structure for SurrealKV checkpoints,
//! including serialization, verification, and consistency checks.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Metadata for a checkpoint created from current state
///
/// This structure maintains the invariants needed for:
/// - One-way consistency (applied_index = checkpoint applied_index)
/// - Data integrity (CRC32 checksum)
/// - Recovery idempotency (timestamp for deduplication)
/// - SurrealKV alignment (sequence_number verification)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMetadata {
    /// Applied index when checkpoint was created
    pub applied_index: u64,

    /// Term when checkpoint was created
    pub term: u64,

    /// SurrealKV sequence number for this checkpoint
    /// Used to verify data consistency with SurrealKV state
    pub sequence_number: u64,

    /// Timestamp (Unix seconds) when checkpoint was created
    pub timestamp: u64,

    /// CRC32 checksum of checkpoint data
    /// Used to verify data integrity during decompression
    pub crc32: u32,

    /// Path to the checkpoint directory
    pub checkpoint_dir: PathBuf,

    /// Size of the compressed snapshot in bytes
    pub compressed_size: u64,
}

impl CheckpointMetadata {
    /// Create a new checkpoint metadata
    pub fn new(
        applied_index: u64,
        term: u64,
        sequence_number: u64,
        checkpoint_dir: PathBuf,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            applied_index,
            term,
            sequence_number,
            timestamp,
            crc32: 0,
            checkpoint_dir,
            compressed_size: 0,
        }
    }

    /// Set CRC32 checksum
    pub fn with_crc32(mut self, crc32: u32) -> Self {
        self.crc32 = crc32;
        self
    }

    /// Set compressed size
    pub fn with_compressed_size(mut self, size: u64) -> Self {
        self.compressed_size = size;
        self
    }

    /// Verify consistency with expected applied_index
    pub fn verify_consistency(&self, expected_index: u64) -> bool {
        self.applied_index == expected_index
    }

    /// Serialize metadata to bytes (postcard format)
    pub fn serialize(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    /// Deserialize metadata from bytes (postcard format)
    pub fn deserialize(data: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(data)
    }

    /// Generate checkpoint directory name for this checkpoint
    pub fn dir_name(&self) -> String {
        format!("checkpoint_{}", self.timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_metadata_creation() {
        let meta = CheckpointMetadata::new(100, 5, 12345, PathBuf::from("/tmp/checkpoint"));

        assert_eq!(meta.applied_index, 100);
        assert_eq!(meta.term, 5);
        assert_eq!(meta.sequence_number, 12345);
        assert!(meta.timestamp > 0);
    }

    #[test]
    fn test_checkpoint_metadata_with_builder() {
        let meta = CheckpointMetadata::new(100, 5, 12345, PathBuf::from("/tmp/checkpoint"))
            .with_crc32(0xdeadbeef)
            .with_compressed_size(1024000);

        assert_eq!(meta.crc32, 0xdeadbeef);
        assert_eq!(meta.compressed_size, 1024000);
    }

    #[test]
    fn test_checkpoint_metadata_serialization() {
        let original = CheckpointMetadata::new(100, 5, 12345, PathBuf::from("/tmp/checkpoint"))
            .with_crc32(0xdeadbeef)
            .with_compressed_size(1024000);

        let serialized = original.serialize().unwrap();
        let deserialized = CheckpointMetadata::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.applied_index, 100);
        assert_eq!(deserialized.term, 5);
        assert_eq!(deserialized.crc32, 0xdeadbeef);
        assert_eq!(deserialized.compressed_size, 1024000);
    }

    #[test]
    fn test_checkpoint_metadata_consistency_verification() {
        let meta = CheckpointMetadata::new(100, 5, 12345, PathBuf::from("/tmp/checkpoint"));

        assert!(meta.verify_consistency(100));
        assert!(!meta.verify_consistency(99));
        assert!(!meta.verify_consistency(101));
    }

    #[test]
    fn test_checkpoint_metadata_dir_name() {
        let meta = CheckpointMetadata::new(100, 5, 12345, PathBuf::from("/tmp/checkpoint"));
        let dir_name = meta.dir_name();

        assert!(dir_name.starts_with("checkpoint_"));
        assert!(dir_name.len() > "checkpoint_".len());
    }
}
