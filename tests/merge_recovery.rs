use openraft_surrealkv::error::Result;
use openraft_surrealkv::state::{
    CheckpointMetadata, DeltaInfo, MergeProgressState, SnapshotMetaState,
};
use openraft_surrealkv::storage::SurrealStorage;
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tempfile::TempDir;

#[tokio::test]
async fn test_merge_recovery_clears_exhausted_retries() -> Result<()> {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree").into())
            .build()
            .unwrap(),
    );

    // 模拟崩溃前状态：合并进行中，已达最大重试
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

    // 重新创建 storage，启用合并策略
    let storage_recovered = SurrealStorage::new(tree)
        .await?
        .with_default_merge("test_node");

    // 执行恢复
    storage_recovered.recover_merge_state().await?;

    // 验证状态被清空
    let final_progress = storage_recovered
        .metadata()
        .get_merge_progress_state()
        .await;
    assert!(!final_progress.in_progress);
    assert_eq!(final_progress.attempt, 0);

    Ok(())
}

#[tokio::test]
async fn test_merge_recovery_spawns_if_policy_met() -> Result<()> {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree_recovery").into())
            .build()
            .unwrap(),
    );

    let storage = SurrealStorage::new(tree.clone()).await?;

    // 设置满足合并策略的快照状态
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

    // 模拟崩溃前进行中的合并（未达最大重试）
    let progress = MergeProgressState::started("chain_too_long".to_string(), 1, 3, 100);
    storage
        .metadata()
        .save_merge_progress_state(progress)
        .await?;

    // 重新创建 storage 并启用合并
    let storage_recovered = SurrealStorage::new(tree)
        .await?
        .with_default_merge("test_node");

    // 执行恢复
    storage_recovered.recover_merge_state().await?;

    // 验证：由于策略满足，应该已重新启动合并
    // （此测试不等待合并完成，仅验证恢复逻辑不报错）
    Ok(())
}

#[tokio::test]
async fn test_merge_recovery_skips_when_policy_not_met() -> Result<()> {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree_no_recovery").into())
            .build()
            .unwrap(),
    );

    let storage = SurrealStorage::new(tree.clone()).await?;

    // 设置不满足合并策略的快照状态
    let snapshot_state = SnapshotMetaState::new();
    storage
        .metadata()
        .save_snapshot_state(snapshot_state)
        .await?;

    // 模拟崩溃前进行中的合并
    let progress = MergeProgressState::started("time_window_expired".to_string(), 1, 3, 100);
    storage
        .metadata()
        .save_merge_progress_state(progress)
        .await?;

    // 重新创建 storage 并启用合并
    let storage_recovered = SurrealStorage::new(tree)
        .await?
        .with_default_merge("test_node");

    // 执行恢复
    storage_recovered.recover_merge_state().await?;

    // 验证：策略不满足，状态应被清除
    let final_progress = storage_recovered
        .metadata()
        .get_merge_progress_state()
        .await;
    assert!(!final_progress.in_progress);

    Ok(())
}

#[tokio::test]
async fn test_merge_recovery_without_executor_configured() -> Result<()> {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree_no_executor").into())
            .build()
            .unwrap(),
    );

    let storage = SurrealStorage::new(tree.clone()).await?;

    // 模拟崩溃前进行中的合并
    let progress = MergeProgressState::started("chain_too_long".to_string(), 1, 3, 100);
    storage
        .metadata()
        .save_merge_progress_state(progress)
        .await?;

    // 重新创建 storage，但不启用合并策略
    let storage_recovered = SurrealStorage::new(tree).await?;

    // 执行恢复（无 executor，应记录警告但不报错）
    storage_recovered.recover_merge_state().await?;

    Ok(())
}
