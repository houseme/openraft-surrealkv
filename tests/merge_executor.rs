use async_trait::async_trait;
use openraft_surrealkv::error::{Error, Result};
use openraft_surrealkv::merge::{
    DeltaMergePolicy, MergeBackend, MergeCleanup, MergeCleanupConfig, MergeExecution,
    MergeExecutor, MERGE_ERR_INJECTED_FAILURE,
};
use openraft_surrealkv::metrics::MergeMetrics;
use openraft_surrealkv::state::{
    CheckpointMetadata, DeltaInfo, MetadataManager, SnapshotMetaState,
};
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tempfile::TempDir;
use tokio::sync::Mutex;

#[derive(Debug)]
struct FlakyBackend {
    remaining_failures: Arc<Mutex<u8>>,
}

#[async_trait]
impl MergeBackend for FlakyBackend {
    async fn execute_merge(&self, _snapshot_state: &SnapshotMetaState) -> Result<MergeExecution> {
        let mut guard = self.remaining_failures.lock().await;
        if *guard > 0 {
            *guard -= 1;
            return Err(Error::Snapshot(format!(
                "{}: injected failure",
                MERGE_ERR_INJECTED_FAILURE
            )));
        }
        Ok(MergeExecution {
            checkpoint_size_bytes: 8192,
            checkpoint_path: Some("target/checkpoints/checkpoint_test".to_string()),
        })
    }
}

#[tokio::test]
async fn merge_executor_retries_and_updates_state() {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree").into())
            .build()
            .unwrap(),
    );

    let metadata = Arc::new(MetadataManager::new(tree));
    let mut state = SnapshotMetaState::new();
    state.last_checkpoint = Some(CheckpointMetadata::new(10, 2, 3, 1));
    for i in 0..5 {
        state
            .delta_chain
            .push(DeltaInfo::new(i * 10, i * 10 + 9, 100, 2));
    }
    state.total_delta_bytes = 500;
    metadata.save_snapshot_state(state).await.unwrap();

    let backend = Arc::new(FlakyBackend {
        remaining_failures: Arc::new(Mutex::new(2)),
    });

    let executor = MergeExecutor::new(
        metadata.clone(),
        DeltaMergePolicy::default(),
        MergeMetrics::new("1"),
        MergeCleanup::new(MergeCleanupConfig::default()),
    )
    .with_backend(backend)
    .with_max_retries(3);

    let task = executor.spawn_if_needed().await.unwrap().unwrap();
    let result = task.handle.await.unwrap().unwrap();

    assert!(result.merged);
    assert_eq!(result.retries, 3);

    let snapshot_state = metadata.get_snapshot_state().await;
    assert!(snapshot_state.delta_chain.is_empty());
    assert_eq!(snapshot_state.total_delta_bytes, 0);

    let progress = metadata.get_merge_progress_state().await;
    assert!(!progress.in_progress);
    assert_eq!(progress.attempt, 3);
}

#[tokio::test]
async fn merge_executor_does_not_spawn_when_policy_not_triggered() {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree_no_spawn").into())
            .build()
            .unwrap(),
    );

    let metadata = Arc::new(MetadataManager::new(tree));
    metadata
        .save_snapshot_state(SnapshotMetaState::new())
        .await
        .unwrap();

    let executor = MergeExecutor::new(
        metadata,
        DeltaMergePolicy::default(),
        MergeMetrics::new("1"),
        MergeCleanup::new(MergeCleanupConfig::default()),
    );

    let task = executor.spawn_if_needed().await.unwrap();
    assert!(task.is_none());
}
