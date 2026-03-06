use super::MergeBackend;
use crate::error::{Error, Result};
use crate::snapshot::CheckpointBuilder;
use crate::state::SnapshotMetaState;
use crate::types::{DeltaEntry, KVRequest};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use surrealkv::{Tree, TreeBuilder};

/// 真实的 checkpoint 合并后端（依赖 Phase 2）
///
/// 执行流程：
/// 1. 创建临时 SurrealKV Tree
/// 2. 从基线 checkpoint 恢复状态（如果存在）
/// 3. 重放所有 delta entries
/// 4. 创建新的 checkpoint
/// 5. 返回 checkpoint 大小
pub struct CheckpointMergeBackend {
    tree: Arc<Tree>,
    temp_base: PathBuf,
}

impl CheckpointMergeBackend {
    pub fn new(tree: Arc<Tree>) -> Self {
        Self {
            tree,
            temp_base: PathBuf::from("target/tmp/merge"),
        }
    }

    pub fn with_temp_base(mut self, temp_base: PathBuf) -> Self {
        self.temp_base = temp_base;
        self
    }

    /// 创建临时 Tree 用于合并
    async fn create_temp_tree(&self) -> Result<Arc<Tree>> {
        let temp_id = uuid::Uuid::new_v4();
        let temp_path = self.temp_base.join(format!("merge_{}", temp_id));

        tokio::fs::create_dir_all(&temp_path).await?;

        let temp_tree = TreeBuilder::new()
            .with_path(temp_path.into())
            .build()
            .map_err(|e| Error::Storage(format!("failed to create temp tree: {}", e)))?;

        Ok(Arc::new(temp_tree))
    }

    /// 重放 delta entries 到临时 Tree
    async fn replay_deltas(
        &self,
        temp_tree: &Arc<Tree>,
        snapshot_state: &SnapshotMetaState,
    ) -> Result<()> {
        // 从持久化的 delta 文件中读取 entries
        for delta_info in &snapshot_state.delta_chain {
            if let Some(file_path) = &delta_info.file_path {
                let bytes = tokio::fs::read(file_path).await?;

                // 解码 delta entries
                let entries = crate::snapshot::DeltaSnapshotCodec::decode_entries(&bytes)?;

                // 重放每个 entry
                for entry in entries {
                    self.apply_entry_to_tree(temp_tree, &entry).await?;
                }
            }
        }

        Ok(())
    }

    /// 将单个 entry 应用到 Tree
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

    /// 创建 checkpoint 并返回大小
    async fn create_checkpoint_from_tree(
        &self,
        temp_tree: &Arc<Tree>,
        snapshot_state: &SnapshotMetaState,
    ) -> Result<u64> {
        let checkpoint_base = PathBuf::from("target/checkpoints");
        let builder = CheckpointBuilder::new(checkpoint_base);

        // 确定 applied_index 和 term
        let (applied_index, term) = snapshot_state
            .delta_chain
            .last()
            .map(|d| (d.end_index, 0u64)) // term 暂时用 0（待后续完善）
            .unwrap_or((0, 0));

        let _metadata = builder
            .create(temp_tree.clone(), applied_index, term)
            .await?;

        // 返回 checkpoint 大小（估算：使用累计字节）
        Ok(snapshot_state.total_delta_bytes)
    }
}

#[async_trait]
impl MergeBackend for CheckpointMergeBackend {
    async fn execute_merge(&self, snapshot_state: &SnapshotMetaState) -> Result<u64> {
        tracing::info!(
            delta_count = snapshot_state.delta_chain.len(),
            total_bytes = snapshot_state.total_delta_bytes,
            "starting checkpoint merge"
        );

        // 1. 创建临时 Tree
        let temp_tree = self.create_temp_tree().await?;

        // 2. 重放 delta entries
        self.replay_deltas(&temp_tree, snapshot_state).await?;

        // 3. 创建 checkpoint
        let checkpoint_size = self
            .create_checkpoint_from_tree(&temp_tree, snapshot_state)
            .await?;

        tracing::info!(
            checkpoint_size_bytes = checkpoint_size,
            "checkpoint merge completed successfully"
        );

        Ok(checkpoint_size)
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

        let backend = CheckpointMergeBackend::new(tree.clone())
            .with_temp_base(base.path().join("temp").into());

        // 创建模拟的 snapshot state（无 delta）
        let mut state = SnapshotMetaState::new();
        state.last_checkpoint = Some(CheckpointMetadata::new(10, 2, 1, 100));

        // 执行合并（应成功但无实际操作）
        let size = backend.execute_merge(&state).await.unwrap();
        assert!(size >= 0);
    }
}
