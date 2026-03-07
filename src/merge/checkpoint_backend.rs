use super::{
    MergeBackend, MergeExecution, MERGE_ERR_BASELINE_MISSING,
    MERGE_ERR_BASELINE_PATH_MISSING, MERGE_ERR_BASELINE_PATH_REQUIRED,
};
use crate::error::{Error, Result};
use crate::snapshot::CheckpointBuilder;
use crate::state::{CheckpointMetadata, SnapshotMetaState};
use crate::types::{DeltaEntry, KVRequest};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use surrealkv::{Tree, TreeBuilder};

fn merge_snapshot_error(code: &str, detail: impl Into<String>) -> Error {
    Error::Snapshot(format!("{}: {}", code, detail.into()))
}

/// Real checkpoint merge backend (depends on Phase 2).
///
/// Execution flow:
/// 1. Create a temporary SurrealKV tree
/// 2. Restore state from the baseline checkpoint (if present)
/// 3. Replay all delta entries
/// 4. Create a new checkpoint
/// 5. Return checkpoint size
pub struct CheckpointMergeBackend {
    temp_base: PathBuf,
}

impl CheckpointMergeBackend {
    pub fn new() -> Self {
        Self {
            temp_base: PathBuf::from("target/tmp/merge"),
        }
    }

    pub fn with_temp_base(mut self, temp_base: PathBuf) -> Self {
        self.temp_base = temp_base;
        self
    }

    /// Resolve the latest full-checkpoint path (strict mode: missing path is an error).
    async fn resolve_baseline_checkpoint(
        &self,
        snapshot_state: &SnapshotMetaState,
    ) -> Result<PathBuf> {
        let cp = snapshot_state.last_checkpoint.as_ref().ok_or_else(|| {
            merge_snapshot_error(
                MERGE_ERR_BASELINE_MISSING,
                "missing baseline checkpoint metadata; merge aborted",
            )
        })?;

        let path = cp.checkpoint_path.as_ref().ok_or_else(|| {
            merge_snapshot_error(
                MERGE_ERR_BASELINE_PATH_REQUIRED,
                "checkpoint_path is required in strict mode",
            )
        })?;

        let explicit = PathBuf::from(path);
        if tokio::fs::metadata(&explicit).await.is_ok() {
            return Ok(explicit);
        }

        Err(merge_snapshot_error(
            MERGE_ERR_BASELINE_PATH_MISSING,
            format!("baseline checkpoint path missing: {}", explicit.display()),
        ))
    }

    fn copy_dir_recursive_hardlink_or_copy_sync(src: &Path, dst: &Path) -> Result<()> {
        std::fs::create_dir_all(dst)?;

        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            let ty = entry.file_type()?;

            if ty.is_dir() {
                Self::copy_dir_recursive_hardlink_or_copy_sync(&src_path, &dst_path)?;
            } else if ty.is_file() {
                if std::fs::hard_link(&src_path, &dst_path).is_err() {
                    std::fs::copy(&src_path, &dst_path)?;
                }
            }
        }

        Ok(())
    }

    /// Copy baseline checkpoint into a temp directory and open it as a tree.
    async fn load_baseline_checkpoint_to_tree(
        &self,
        checkpoint_path: &Path,
        temp_path: &Path,
    ) -> Result<Arc<Tree>> {
        let checkpoint_path = checkpoint_path.to_path_buf();
        let temp_path_buf = temp_path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            Self::copy_dir_recursive_hardlink_or_copy_sync(&checkpoint_path, &temp_path_buf)
        })
        .await
        .map_err(|e| Error::Storage(format!("baseline copy task panicked: {}", e)))??;

        let tree = TreeBuilder::new()
            .with_path(temp_path.to_path_buf().into())
            .build()
            .map_err(|e| {
                Error::Storage(format!("failed to open temp tree from checkpoint: {}", e))
            })?;

        Ok(Arc::new(tree))
    }

    /// Create a temp tree for merge (strict: baseline checkpoint must be loaded first).
    async fn create_temp_tree(&self, snapshot_state: &SnapshotMetaState) -> Result<Arc<Tree>> {
        let temp_id = uuid::Uuid::new_v4();
        let temp_path = self.temp_base.join(format!("merge_{}", temp_id));
        tokio::fs::create_dir_all(&temp_path).await?;

        let baseline = self.resolve_baseline_checkpoint(snapshot_state).await?;
        self.load_baseline_checkpoint_to_tree(&baseline, &temp_path)
            .await
    }

    /// Replay delta entries into the temporary tree.
    async fn replay_deltas(
        &self,
        temp_tree: &Arc<Tree>,
        snapshot_state: &SnapshotMetaState,
    ) -> Result<()> {
        // Read entries from persisted delta files.
        for delta_info in &snapshot_state.delta_chain {
            if let Some(file_path) = &delta_info.file_path {
                let bytes = tokio::fs::read(file_path).await?;

                // Decode delta entries.
                let entries = crate::snapshot::DeltaSnapshotCodec::decode_entries(&bytes)?;

                // Replay each entry.
                for entry in entries {
                    self.apply_entry_to_tree(temp_tree, &entry).await?;
                }
            }
        }

        Ok(())
    }

    /// Apply a single entry to the tree.
    async fn apply_entry_to_tree(&self, tree: &Arc<Tree>, entry: &DeltaEntry) -> Result<()> {
        let req: KVRequest = postcard::from_bytes(&entry.payload)
            .map_err(|e| Error::Serialization(format!("failed to decode entry: {}", e)))?;

        match req {
            KVRequest::Set { key, value } => {
                let mut txn = tree.begin()?;
                txn.set(key.as_bytes(), &value)?;
                txn.commit().await?;
            }
            KVRequest::Delete { key } => {
                let mut txn = tree.begin()?;
                txn.delete(key.as_bytes())?;
                txn.commit().await?;
            }
        }

        Ok(())
    }

    /// Create a checkpoint and return its size.
    async fn create_checkpoint_from_tree(
        &self,
        temp_tree: &Arc<Tree>,
        snapshot_state: &SnapshotMetaState,
    ) -> Result<MergeExecution> {
        let checkpoint_base = PathBuf::from("target/checkpoints");
        let builder = CheckpointBuilder::new(checkpoint_base);

        // Prefer the last delta end index; if no deltas exist, inherit last_checkpoint.
        let (applied_index, term) = if let Some(last_delta) = snapshot_state.delta_chain.last() {
            (
                last_delta.end_index,
                snapshot_state
                    .last_checkpoint
                    .as_ref()
                    .map(|cp| cp.checkpoint_term)
                    .unwrap_or(0),
            )
        } else if let Some(CheckpointMetadata {
            checkpoint_index,
            checkpoint_term,
            ..
        }) = snapshot_state.last_checkpoint.as_ref()
        {
            (*checkpoint_index, *checkpoint_term)
        } else {
            (0, 0)
        };

        let metadata = builder
            .create(temp_tree.clone(), applied_index, term)
            .await?;
        let size = builder.get_checkpoint_size(&metadata).await?;
        let checkpoint_path = metadata
            .checkpoint_dir
            .join(metadata.dir_name())
            .to_string_lossy()
            .to_string();

        Ok(MergeExecution {
            checkpoint_size_bytes: size.max(snapshot_state.total_delta_bytes),
            checkpoint_path: Some(checkpoint_path),
        })
    }
}

impl Default for CheckpointMergeBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MergeBackend for CheckpointMergeBackend {
    async fn execute_merge(&self, snapshot_state: &SnapshotMetaState) -> Result<MergeExecution> {
        tracing::info!(
            delta_count = snapshot_state.delta_chain.len(),
            total_bytes = snapshot_state.total_delta_bytes,
            "starting checkpoint merge"
        );

        // 1. Create temporary tree (strict mode requires an available baseline checkpoint).
        let temp_tree = self.create_temp_tree(snapshot_state).await?;

        // 2. Replay delta entries on top of baseline.
        self.replay_deltas(&temp_tree, snapshot_state).await?;

        // 3. Create checkpoint.
        let exec = self
            .create_checkpoint_from_tree(&temp_tree, snapshot_state)
            .await?;

        tracing::info!(
            checkpoint_size_bytes = exec.checkpoint_size_bytes,
            checkpoint_path = exec.checkpoint_path.as_deref().unwrap_or("unknown"),
            "checkpoint merge completed successfully"
        );

        Ok(exec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::CheckpointMetadata;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_checkpoint_merge_backend_basic() {
        let base = TempDir::new().unwrap();
        let tree = Arc::new(
            TreeBuilder::new()
                .with_path(base.path().join("tree").into())
                .build()
                .unwrap(),
        );

        // Prepare a real baseline checkpoint first in strict mode.
        let baseline_path = base.path().join("baseline_cp");
        let tree_for_cp = tree.clone();
        let baseline_path_for_cp = baseline_path.clone();
        tokio::task::spawn_blocking(move || tree_for_cp.create_checkpoint(&baseline_path_for_cp))
            .await
            .unwrap()
            .unwrap();

        let backend = CheckpointMergeBackend::new().with_temp_base(base.path().join("temp").into());

        // Create a mock snapshot state (no delta).
        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(
            CheckpointMetadata::new(10, 2, 1, 100)
                .with_checkpoint_path(baseline_path.to_string_lossy().to_string()),
        );

        // Execute merge (should succeed with no effective replay work).
        let size = backend.execute_merge(&state).await.unwrap();
        assert!(size.checkpoint_size_bytes > 0);
    }

    #[tokio::test]
    async fn test_checkpoint_merge_backend_fail_fast_without_checkpoint_meta() {
        let base = TempDir::new().unwrap();
        let backend = CheckpointMergeBackend::new().with_temp_base(base.path().join("temp").into());

        let state = SnapshotMetaState::new();
        let err = backend.execute_merge(&state).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(MERGE_ERR_BASELINE_MISSING));
    }

    #[tokio::test]
    async fn test_checkpoint_merge_backend_fail_fast_missing_explicit_checkpoint_path() {
        let base = TempDir::new().unwrap();
        let backend = CheckpointMergeBackend::new().with_temp_base(base.path().join("temp").into());

        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(
            CheckpointMetadata::new(10, 2, 1, 100)
                .with_checkpoint_path(base.path().join("not-exist").to_string_lossy().to_string()),
        );

        let err = backend.execute_merge(&state).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(MERGE_ERR_BASELINE_PATH_MISSING));
    }

    #[tokio::test]
    async fn test_checkpoint_merge_backend_fail_fast_when_checkpoint_path_is_absent() {
        let base = TempDir::new().unwrap();
        let backend = CheckpointMergeBackend::new().with_temp_base(base.path().join("temp").into());

        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(CheckpointMetadata::new(10, 2, 1, 100));

        let err = backend.execute_merge(&state).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(MERGE_ERR_BASELINE_PATH_REQUIRED));
    }
}
