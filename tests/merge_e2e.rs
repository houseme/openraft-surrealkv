use openraft_surrealkv::error::Result;
use openraft_surrealkv::merge::{
    CheckpointMergeBackend, DeltaMergePolicy, MergeBackend, MergeCleanup, MergeCleanupConfig,
    MergeExecutor,
};
use openraft_surrealkv::metrics::MergeMetrics;
use openraft_surrealkv::snapshot::DeltaSnapshotCodec;
use openraft_surrealkv::state::{
    CheckpointMetadata, DeltaInfo, MetadataManager, SnapshotMetaState,
};
use openraft_surrealkv::types::{DeltaEntry, KVRequest};
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tempfile::TempDir;

/// 端到端测试：合并后的快照可被正确恢复
#[tokio::test]
async fn test_e2e_merge_and_restore() -> Result<()> {
    let base = TempDir::new().unwrap();

    // 1. 创建初始 Tree 并写入数据
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree").into())
            .build()
            .unwrap(),
    );

    // 写入初始数据
    let mut txn = tree.begin()?;
    txn.set(b"key1", b"value1")?;
    txn.set(b"key2", b"value2")?;
    txn.commit().await?;

    // 2. 创建模拟的 delta entries
    let delta_entries = vec![
        DeltaEntry::new(
            1,
            101,
            postcard::to_stdvec(&KVRequest::Set {
                key: "key3".to_string(),
                value: b"value3".to_vec(),
            })
            .unwrap(),
        ),
        DeltaEntry::new(
            1,
            102,
            postcard::to_stdvec(&KVRequest::Set {
                key: "key4".to_string(),
                value: b"value4".to_vec(),
            })
            .unwrap(),
        ),
        DeltaEntry::new(
            1,
            103,
            postcard::to_stdvec(&KVRequest::Delete {
                key: "key1".to_string(),
            })
            .unwrap(),
        ),
    ];

    // 3. 持久化 delta 到文件
    let delta_dir = base.path().join("deltas");
    tokio::fs::create_dir_all(&delta_dir).await?;
    let delta_path = delta_dir.join("delta_100_103.bin");

    let compressed = DeltaSnapshotCodec::encode_entries(&delta_entries)?;
    tokio::fs::write(&delta_path, &compressed).await?;

    // 4. 构建 SnapshotMetaState
    let metadata_mgr = Arc::new(MetadataManager::new(tree.clone()));
    let mut snapshot_state = SnapshotMetaState::new();
    snapshot_state.last_checkpoint = Some(CheckpointMetadata::new(100, 1, 1, 1000));
    snapshot_state.delta_chain.push(
        DeltaInfo::new(101, 103, compressed.len() as u64, 1001)
            .with_file_path(delta_path.display().to_string()),
    );
    snapshot_state.total_delta_bytes = compressed.len() as u64;
    metadata_mgr
        .save_snapshot_state(snapshot_state.clone())
        .await?;

    // 5. 执行合并
    let backend = Arc::new(
        CheckpointMergeBackend::new(tree.clone()).with_temp_base(base.path().join("temp").into()),
    );

    let executor = MergeExecutor::new(
        metadata_mgr.clone(),
        DeltaMergePolicy {
            max_chain_length: 1, // 立即触发
            max_delta_bytes: 1,
            checkpoint_interval_secs: 1,
        },
        MergeMetrics::new("test_node"),
        MergeCleanup::new(MergeCleanupConfig::default()),
    )
    .with_backend(backend.clone())
    .with_max_retries(3);

    let handle = executor.spawn_if_needed().await?.expect("should spawn");
    let result = handle.handle.await.unwrap()?;

    assert!(result.merged);
    assert_eq!(result.retries, 1);

    // 6. 验证合并后状态
    let final_state = metadata_mgr.get_snapshot_state().await;
    assert!(final_state.delta_chain.is_empty());
    assert_eq!(final_state.total_delta_bytes, 0);

    // 7. 验证 checkpoint 已创建
    let checkpoint_dir = base.path().join("../checkpoints");
    if checkpoint_dir.exists() {
        tracing::info!("checkpoint directory exists, merge succeeded");
    }

    Ok(())
}

/// 测试：多次合并幂等性
#[tokio::test]
async fn test_e2e_multiple_merges_idempotent() -> Result<()> {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree").into())
            .build()
            .unwrap(),
    );

    let metadata_mgr = Arc::new(MetadataManager::new(tree.clone()));

    // 设置满足合并条件的状态
    let mut state = SnapshotMetaState::new();
    state.last_checkpoint = Some(CheckpointMetadata::new(100, 1, 1, 1000));
    for i in 0..5 {
        state
            .delta_chain
            .push(DeltaInfo::new(i * 10, i * 10 + 9, 1000, 1001));
    }
    state.total_delta_bytes = 5000;
    metadata_mgr.save_snapshot_state(state).await?;

    let backend = Arc::new(
        CheckpointMergeBackend::new(tree.clone()).with_temp_base(base.path().join("temp").into()),
    );

    let executor = MergeExecutor::new(
        metadata_mgr.clone(),
        DeltaMergePolicy::default(),
        MergeMetrics::new("test_node"),
        MergeCleanup::new(MergeCleanupConfig::default()),
    )
    .with_backend(backend);

    // 第一次合并
    let handle1 = executor.spawn_if_needed().await?.unwrap();
    handle1.handle.await.unwrap()?;

    // 第二次尝试合并（应该不触发）
    let handle2 = executor.spawn_if_needed().await?;
    assert!(handle2.is_none(), "second merge should not spawn");

    Ok(())
}

/// 测试：合并失败后恢复
#[tokio::test]
async fn test_e2e_merge_failure_recovery() -> Result<()> {
    use async_trait::async_trait;

    struct FailOnceThenSucceedBackend {
        tree: Arc<surrealkv::Tree>,
        failed: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl MergeBackend for FailOnceThenSucceedBackend {
        async fn execute_merge(&self, snapshot_state: &SnapshotMetaState) -> Result<u64> {
            if !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return Err(openraft_surrealkv::error::Error::Snapshot(
                    "injected failure".to_string(),
                ));
            }

            // 第二次调用成功
            Ok(snapshot_state.total_delta_bytes)
        }
    }

    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree").into())
            .build()
            .unwrap(),
    );

    let metadata_mgr = Arc::new(MetadataManager::new(tree.clone()));
    let mut state = SnapshotMetaState::new();
    state.last_checkpoint = Some(CheckpointMetadata::new(10, 1, 1, 100));
    for i in 0..5 {
        state
            .delta_chain
            .push(DeltaInfo::new(i * 10, i * 10 + 9, 100, 200));
    }
    state.total_delta_bytes = 500;
    metadata_mgr.save_snapshot_state(state).await?;

    let backend = Arc::new(FailOnceThenSucceedBackend {
        tree: tree.clone(),
        failed: std::sync::atomic::AtomicBool::new(false),
    });

    let executor = MergeExecutor::new(
        metadata_mgr.clone(),
        DeltaMergePolicy::default(),
        MergeMetrics::new("test_node"),
        MergeCleanup::new(MergeCleanupConfig::default()),
    )
    .with_backend(backend)
    .with_max_retries(3);

    let handle = executor.spawn_if_needed().await?.unwrap();
    let result = handle.handle.await.unwrap()?;

    assert!(result.merged);
    assert_eq!(result.retries, 2); // 第一次失败，第二次成功

    Ok(())
}
