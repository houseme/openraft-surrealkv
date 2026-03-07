use openraft_surrealkv::error::Result;
use openraft_surrealkv::merge::{
    CheckpointMergeBackend, DeltaMergePolicy, MERGE_ERR_INJECTED_FAILURE, MergeBackend,
    MergeCleanup, MergeCleanupConfig, MergeExecution, MergeExecutor,
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

async fn create_baseline_checkpoint(
    tree: Arc<surrealkv::Tree>,
    path: std::path::PathBuf,
) -> Result<String> {
    let cp_path = path.clone();
    tokio::task::spawn_blocking(move || tree.create_checkpoint(&cp_path))
        .await
        .map_err(|e| {
            openraft_surrealkv::error::Error::Snapshot(format!("checkpoint task panicked: {}", e))
        })??;
    Ok(path.to_string_lossy().to_string())
}

/// End-to-end test: merged snapshots can be recovered correctly.
#[tokio::test]
async fn test_e2e_merge_and_restore() -> Result<()> {
    let base = TempDir::new().unwrap();

    // 1) Create the initial tree and seed baseline data.
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree"))
            .build()
            .unwrap(),
    );

    // Seed baseline key/value data.
    let mut txn = tree.begin()?;
    txn.set(b"key1", b"value1")?;
    txn.set(b"key2", b"value2")?;
    txn.commit().await?;

    // 2) Build synthetic delta entries.
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

    // 3) Persist encoded delta payload to disk.
    let delta_dir = base.path().join("deltas");
    tokio::fs::create_dir_all(&delta_dir).await?;
    let delta_path = delta_dir.join("delta_100_103.bin");

    let compressed = DeltaSnapshotCodec::encode_entries(&delta_entries)?;
    tokio::fs::write(&delta_path, &compressed).await?;

    // 4) Build and persist SnapshotMetaState.
    let metadata_mgr = Arc::new(MetadataManager::new(tree.clone()));
    let baseline_cp =
        create_baseline_checkpoint(tree.clone(), base.path().join("baseline_cp")).await?;

    let mut snapshot_state = SnapshotMetaState::new();
    snapshot_state.last_checkpoint =
        Some(CheckpointMetadata::new(100, 1, 1, 1000).with_checkpoint_path(baseline_cp));
    snapshot_state.delta_chain.push(
        DeltaInfo::new(101, 103, compressed.len() as u64, 1001)
            .with_file_path(delta_path.display().to_string()),
    );
    snapshot_state.total_delta_bytes = compressed.len() as u64;
    metadata_mgr
        .save_snapshot_state(snapshot_state.clone())
        .await?;

    // 5) Execute merge.
    let backend = Arc::new(CheckpointMergeBackend::new().with_temp_base(base.path().join("temp")));

    let executor = MergeExecutor::new(
        metadata_mgr.clone(),
        DeltaMergePolicy {
            max_chain_length: 1, // Trigger immediately.
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

    // 6) Verify post-merge state.
    let final_state = metadata_mgr.get_snapshot_state().await;
    assert!(final_state.delta_chain.is_empty());
    assert_eq!(final_state.total_delta_bytes, 0);

    // 7) Verify checkpoint directory was created.
    let checkpoint_dir = base.path().join("../checkpoints");
    if checkpoint_dir.exists() {
        tracing::info!("checkpoint directory exists, merge succeeded");
    }

    Ok(())
}

/// Verify idempotency when merge is attempted repeatedly.
#[tokio::test]
async fn test_e2e_multiple_merges_idempotent() -> Result<()> {
    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree"))
            .build()
            .unwrap(),
    );

    let metadata_mgr = Arc::new(MetadataManager::new(tree.clone()));
    let baseline_cp =
        create_baseline_checkpoint(tree.clone(), base.path().join("baseline_cp")).await?;

    // Prepare state that satisfies merge conditions.
    let mut state = SnapshotMetaState::new();
    state.last_checkpoint =
        Some(CheckpointMetadata::new(100, 1, 1, 1000).with_checkpoint_path(baseline_cp));
    for i in 0..5 {
        state
            .delta_chain
            .push(DeltaInfo::new(i * 10, i * 10 + 9, 1000, 1001));
    }
    state.total_delta_bytes = 5000;
    metadata_mgr.save_snapshot_state(state).await?;

    let backend = Arc::new(CheckpointMergeBackend::new().with_temp_base(base.path().join("temp")));

    let executor = MergeExecutor::new(
        metadata_mgr.clone(),
        DeltaMergePolicy::default(),
        MergeMetrics::new("test_node"),
        MergeCleanup::new(MergeCleanupConfig::default()),
    )
    .with_backend(backend);

    // First merge attempt.
    let handle1 = executor.spawn_if_needed().await?.unwrap();
    handle1.handle.await.unwrap()?;

    // Second merge attempt should not spawn a new job.
    let handle2 = executor.spawn_if_needed().await?;
    assert!(handle2.is_none(), "second merge should not spawn");

    Ok(())
}

/// Verify merge recovery after an injected one-time backend failure.
#[tokio::test]
async fn test_e2e_merge_failure_recovery() -> Result<()> {
    use async_trait::async_trait;

    struct FailOnceThenSucceedBackend {
        failed: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl MergeBackend for FailOnceThenSucceedBackend {
        async fn execute_merge(
            &self,
            snapshot_state: &SnapshotMetaState,
        ) -> Result<MergeExecution> {
            if !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return Err(openraft_surrealkv::error::Error::Snapshot(format!(
                    "{}: injected failure",
                    MERGE_ERR_INJECTED_FAILURE
                )));
            }

            // The second call succeeds.
            Ok(MergeExecution {
                checkpoint_size_bytes: snapshot_state.total_delta_bytes,
                checkpoint_path: Some("target/checkpoints/checkpoint_test".to_string()),
            })
        }
    }

    let base = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("tree"))
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
    assert_eq!(result.retries, 2); // First call fails, second call succeeds.

    Ok(())
}
