use openraft_surrealkv::error::Result;
use openraft_surrealkv::state::{
    CheckpointMetadata, DeltaInfo, MergeProgressState, SnapshotMetaState,
};
use openraft_surrealkv::storage::SurrealStorage;
use serial_test::serial;
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tempfile::TempDir;

#[tokio::test]
#[serial]
async fn test_merge_recovery_clears_exhausted_retries() -> Result<()> {
    let base = TempDir::new()?;
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree"))
            .build()?,
    );

    // Simulate pre-crash state: merge was in progress and retries were exhausted.
    let storage = SurrealStorage::new(tree.clone()).await?;
    let progress = MergeProgressState::failed(
        "chain_too_long".to_string(),
        3,
        3,
        100,
        "simulated crash".to_string(),
    );
    storage
        .metadata()
        .save_merge_progress_state(progress)
        .await?;

    // Recreate storage with merge policy enabled.
    let storage_recovered = SurrealStorage::new(tree)
        .await?
        .with_default_merge("test_node");

    // Execute recovery flow.
    storage_recovered.recover_merge_state().await?;

    // Verify progress state is cleared after exhausted retries.
    let final_progress = storage_recovered
        .metadata()
        .get_merge_progress_state()
        .await;
    assert!(!final_progress.in_progress);
    assert_eq!(final_progress.attempt, 3);

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_merge_recovery_spawns_if_policy_met() -> Result<()> {
    let base = TempDir::new()?;
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree_recovery"))
            .build()?,
    );

    let storage = SurrealStorage::new(tree.clone()).await?;

    // Prepare snapshot state that satisfies merge policy thresholds.
    let mut snapshot_state = SnapshotMetaState::new();
    snapshot_state.last_checkpoint = Some(CheckpointMetadata::new(10, 2, 1, 100));
    for i in 0..5 {
        snapshot_state
            .delta_chain
            .push(DeltaInfo::new(i * 10, i * 10 + 9, 1000, 200));
    }
    snapshot_state.total_delta_bytes = 5000;
    storage
        .metadata()
        .save_snapshot_state(snapshot_state)
        .await?;

    // Simulate pre-crash in-progress merge (retries not yet exhausted).
    let progress = MergeProgressState::started("chain_too_long".to_string(), 1, 3, 100);
    storage
        .metadata()
        .save_merge_progress_state(progress)
        .await?;

    // Recreate storage and enable merge executor.
    let storage_recovered = SurrealStorage::new(tree)
        .await?
        .with_default_merge("test_node");

    // Execute recovery flow.
    storage_recovered.recover_merge_state().await?;

    // Policy is satisfied, so recovery should re-trigger merge scheduling.
    // This test only verifies recovery path success, not merge completion.
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_merge_recovery_skips_when_policy_not_met() -> Result<()> {
    let base = TempDir::new()?;
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree_no_recovery"))
            .build()?,
    );

    let storage = SurrealStorage::new(tree.clone()).await?;

    // Prepare snapshot state that does not satisfy merge policy thresholds.
    let snapshot_state = SnapshotMetaState::new();
    storage
        .metadata()
        .save_snapshot_state(snapshot_state)
        .await?;

    // Simulate pre-crash in-progress merge.
    let progress = MergeProgressState::started("time_window_expired".to_string(), 1, 3, 100);
    storage
        .metadata()
        .save_merge_progress_state(progress)
        .await?;

    // Recreate storage and enable merge executor.
    let storage_recovered = SurrealStorage::new(tree)
        .await?
        .with_default_merge("test_node");

    // Execute recovery flow.
    storage_recovered.recover_merge_state().await?;

    // Policy is not satisfied, so in-progress state should be cleared.
    let final_progress = storage_recovered
        .metadata()
        .get_merge_progress_state()
        .await;
    assert!(!final_progress.in_progress);

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_merge_recovery_without_executor_configured() -> Result<()> {
    let base = TempDir::new()?;
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree_no_executor"))
            .build()?,
    );

    let storage = SurrealStorage::new(tree.clone()).await?;

    // Simulate pre-crash in-progress merge.
    let progress = MergeProgressState::started("chain_too_long".to_string(), 1, 3, 100);
    storage
        .metadata()
        .save_merge_progress_state(progress)
        .await?;

    // Recreate storage without enabling merge executor.
    let storage_recovered = SurrealStorage::new(tree).await?;

    // Execute recovery: should warn but not fail when executor is unavailable.
    storage_recovered.recover_merge_state().await?;

    Ok(())
}
