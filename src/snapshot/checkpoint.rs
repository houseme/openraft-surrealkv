//! Checkpoint creation for Phase 2
//!
//! This module implements checkpoint creation using SurrealKV's hardlink mechanism.
//! Checkpoint creation is O(1) - only metadata and hardlinks are created,
//! no data is actually copied.

use super::consistency::{
    PersistedKeySequenceNumberReader, PersistedKeySequenceNumberWriter, resolve_sequence_number,
    write_sequence_number_dual,
};
use super::metadata::CheckpointMetadata;
use crate::error::{Error, Result};
use std::path::PathBuf;
use std::sync::Arc;
use surrealkv::Tree;
use tokio::fs;

/// Builder for creating checkpoints from SurrealKV state
pub struct CheckpointBuilder {
    checkpoint_base_dir: PathBuf,
}

impl CheckpointBuilder {
    /// Create a new checkpoint builder with a base directory
    pub fn new(checkpoint_base_dir: PathBuf) -> Self {
        Self {
            checkpoint_base_dir,
        }
    }

    /// Create a checkpoint from the current state
    ///
    /// This implements the O(1) checkpoint strategy using SurrealKV hardlinks:
    /// 1. Create checkpoint directory with timestamp
    /// 2. Call SurrealKV create_checkpoint() (hardlinks, not copy)
    /// 3. Generate CheckpointMetadata with consistency info
    /// 4. Return metadata for subsequent compression
    ///
    /// # Arguments
    /// * `tree` - Arc<Tree> reference to SurrealKV state
    /// * `applied_index` - Current applied_index for consistency verification
    /// * `term` - Current term
    ///
    /// # Performance
    /// - Time: O(1) - just hardlinks, no data copy
    /// - Disk space: Negligible (hardlinks only)
    pub async fn create(
        &self,
        tree: Arc<Tree>,
        applied_index: u64,
        term: u64,
    ) -> Result<CheckpointMetadata> {
        // Step 1: Create checkpoint directory
        let checkpoint_dir = self.checkpoint_base_dir.clone();
        fs::create_dir_all(&checkpoint_dir).await?;

        let meta = CheckpointMetadata::new(applied_index, term, 0, checkpoint_dir.clone());
        let checkpoint_path = checkpoint_dir.join(meta.dir_name());

        // Step 2: Create directory for this checkpoint
        fs::create_dir_all(&checkpoint_path).await?;

        // Step 3: Call SurrealKV create_checkpoint
        // Note: This is a placeholder - actual implementation depends on SurrealKV API
        // The checkpoint creation should be done on the blocking thread pool
        // to avoid blocking the async runtime
        let tree_cloned = tree.clone();
        let checkpoint_path_cloned = checkpoint_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            tree_cloned.create_checkpoint(&checkpoint_path_cloned)
        })
        .await;

        match result {
            Ok(Ok(db_meta)) => {
                // Resolve sequence number priority:
                // SurrealKV metadata -> checkpoint manifest -> persisted key.
                let reader = PersistedKeySequenceNumberReader;
                let seq = resolve_sequence_number(
                    Some(db_meta.sequence_number),
                    &checkpoint_path,
                    &tree,
                    &reader,
                )?;

                // Enforce dual persistence contract for subsequent recovery/tests.
                let writer = PersistedKeySequenceNumberWriter;
                write_sequence_number_dual(&tree, &checkpoint_path, seq, &writer)?;

                let meta = CheckpointMetadata::new(applied_index, term, seq, checkpoint_dir)
                    .with_compressed_size(0);

                Ok(meta)
            }
            Ok(Err(e)) => Err(Error::Snapshot(format!(
                "Failed to create checkpoint: {}",
                e
            ))),
            Err(e) => Err(Error::Snapshot(format!(
                "Checkpoint creation task panicked: {}",
                e
            ))),
        }
    }

    /// Verify that a checkpoint was created successfully
    pub async fn verify_checkpoint_exists(&self, meta: &CheckpointMetadata) -> Result<bool> {
        let checkpoint_path = meta.checkpoint_dir.join(meta.dir_name());
        match fs::metadata(&checkpoint_path).await {
            Ok(metadata) => Ok(metadata.is_dir()),
            Err(_) => Ok(false),
        }
    }

    /// Get checkpoint size in bytes (sum of all hardlinked files)
    pub async fn get_checkpoint_size(&self, meta: &CheckpointMetadata) -> Result<u64> {
        let checkpoint_path = meta.checkpoint_dir.join(meta.dir_name());

        // Walk directory and sum up file sizes
        let mut total_size = 0u64;
        let mut entries = fs::read_dir(&checkpoint_path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            if metadata.is_file() {
                total_size += metadata.len();
            }
        }

        Ok(total_size)
    }
}

#[cfg(test)]
mod tests {
    use crate::snapshot::CheckpointBuilder;
    use crate::snapshot::CheckpointMetadata;
    use std::sync::Arc;
    use surrealkv::TreeBuilder;
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_checkpoint_builder_creation() {
        let temp_dir = TempDir::new().unwrap();
        let _builder = CheckpointBuilder::new(temp_dir.path().to_path_buf());

        let meta = CheckpointMetadata::new(100, 5, 0, temp_dir.path().to_path_buf());
        let dir_name = meta.dir_name();

        assert!(dir_name.starts_with("checkpoint_"));
    }

    #[tokio::test]
    async fn test_checkpoint_directory_creation() {
        let temp_dir = TempDir::new().unwrap();
        let checkpoint_dir = temp_dir.path().join("checkpoints");

        fs::create_dir_all(&checkpoint_dir).await.unwrap();

        let meta = CheckpointMetadata::new(100, 5, 0, checkpoint_dir.clone());
        let checkpoint_path = checkpoint_dir.join(meta.dir_name());

        fs::create_dir_all(&checkpoint_path).await.unwrap();

        assert!(fs::metadata(&checkpoint_path).await.unwrap().is_dir());
    }

    #[tokio::test]
    async fn test_checkpoint_metadata_consistency() {
        let temp_dir = TempDir::new().unwrap();
        let meta = CheckpointMetadata::new(100, 5, 0, temp_dir.path().to_path_buf());

        assert!(meta.verify_consistency(100));
        assert!(!meta.verify_consistency(99));
    }

    #[tokio::test]
    async fn test_checkpoint_create_with_tree() {
        let temp_dir = TempDir::new().unwrap();
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path(temp_dir.path().join("db").into())
                .build()
                .unwrap(),
        );

        let builder = CheckpointBuilder::new(temp_dir.path().join("checkpoints"));
        let meta = builder.create(tree.clone(), 7, 1).await.unwrap();

        assert!(meta.verify_consistency(7));
        assert!(builder.verify_checkpoint_exists(&meta).await.unwrap());

        // Verify dual persistence: sequence number persisted to SurrealKV key.
        use crate::snapshot::consistency::PersistedKeySequenceNumberReader;
        use crate::snapshot::consistency::SequenceNumberReader;
        let reader = PersistedKeySequenceNumberReader;
        let seq = reader.read_sequence_number(&tree).unwrap();
        assert_eq!(seq, meta.sequence_number);
    }
}
