use openraft_surrealkv::snapshot::{
    DecodedSnapshotPayload, decode_delta_entries_from_payload, decode_snapshot_payload,
};
use openraft_surrealkv::storage::SurrealStorage;
use openraft_surrealkv::types::{KVRequest, SnapshotFormat};
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tempfile::TempDir;

const DELTA_MAX_BYTES: u64 = 5 * 1024 * 1024;

#[tokio::test]
async fn test_auto_build_selects_delta_when_entry_threshold_is_reached() {
    let dir = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(dir.path().join("db"))
            .build()
            .unwrap(),
    );
    let storage = SurrealStorage::new(tree).await.unwrap();

    // First snapshot must be full and establishes baseline.
    let baseline = storage.build_snapshot_auto(1).await.unwrap();
    assert!(baseline.meta.snapshot_id.starts_with("full-"));

    for i in 1..=1000u64 {
        let req = KVRequest::Set {
            key: format!("k{}", i),
            value: vec![i as u8],
        };
        storage.append_log_entry(1, i, &req).await.unwrap();
    }

    let snapshot = storage.build_snapshot_auto(1).await.unwrap();
    assert!(snapshot.meta.snapshot_id.starts_with("delta-"));

    let decoded_payload = decode_snapshot_payload(snapshot.snapshot.get_ref())
        .unwrap()
        .unwrap();
    match decoded_payload {
        DecodedSnapshotPayload::Delta { metadata, .. } => {
            assert_eq!(metadata.version, 2);
            assert!(matches!(
                metadata.format,
                SnapshotFormat::Delta {
                    base_index: 0,
                    entry_count: 1000
                }
            ));
            assert!(metadata.size_bytes < DELTA_MAX_BYTES);
        }
        DecodedSnapshotPayload::Full(_) => panic!("expected delta payload"),
    }

    let decoded = decode_delta_entries_from_payload(snapshot.snapshot.get_ref())
        .unwrap()
        .unwrap();
    assert_eq!(decoded.len(), 1000);
}

#[tokio::test]
async fn test_auto_build_falls_back_to_full_when_no_delta_entries() {
    let dir = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(dir.path().join("db"))
            .build()
            .unwrap(),
    );
    let storage = SurrealStorage::new(tree).await.unwrap();

    let snapshot = storage.build_snapshot_auto(1).await.unwrap();
    assert!(snapshot.meta.snapshot_id.starts_with("full-"));

    let decoded = decode_delta_entries_from_payload(snapshot.snapshot.get_ref()).unwrap();
    assert!(decoded.is_none());
}

#[tokio::test]
async fn test_install_snapshot_auto_replays_delta_entries() {
    let src_dir = TempDir::new().unwrap();
    let src_tree = Arc::new(
        TreeBuilder::new()
            .with_path(src_dir.path().join("db-src"))
            .build()
            .unwrap(),
    );
    let src = SurrealStorage::new(src_tree).await.unwrap();

    // Baseline full snapshot.
    src.build_snapshot_auto(1).await.unwrap();

    for i in 1..=1000u64 {
        let req = KVRequest::Set {
            key: format!("k{}", i),
            value: vec![i as u8],
        };
        src.apply_request(&req).await.unwrap();
        src.append_log_entry(1, i, &req).await.unwrap();
    }

    let snapshot = src.build_snapshot_auto(1).await.unwrap();
    assert!(snapshot.meta.snapshot_id.starts_with("delta-"));

    let dst_dir = TempDir::new().unwrap();
    let dst_tree = Arc::new(
        TreeBuilder::new()
            .with_path(dst_dir.path().join("db-dst"))
            .build()
            .unwrap(),
    );
    let dst = SurrealStorage::new(dst_tree).await.unwrap();

    // Destination also needs matching baseline before delta replay.
    dst.build_snapshot_auto(1).await.unwrap();

    dst.install_snapshot_auto(snapshot).await.unwrap();

    let value = dst.read("k1000").await.unwrap();
    assert_eq!(value, Some(vec![1000u64 as u8]));
}

#[tokio::test]
async fn test_build_snapshot_auto_updates_snapshot_state() {
    let dir = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(dir.path().join("db-meta"))
            .build()
            .unwrap(),
    );
    let storage = SurrealStorage::new(tree).await.unwrap();

    storage.build_snapshot_auto(1).await.unwrap();
    let state_after_full = storage.metadata().get_snapshot_state().await;
    assert!(state_after_full.last_checkpoint.is_some());
    assert!(state_after_full.delta_chain.is_empty());

    for i in 1..=1000u64 {
        let req = KVRequest::Set {
            key: format!("m{}", i),
            value: vec![i as u8],
        };
        storage.append_log_entry(1, i, &req).await.unwrap();
    }

    storage.build_snapshot_auto(1).await.unwrap();
    let state_after_delta = storage.metadata().get_snapshot_state().await;
    assert_eq!(state_after_delta.delta_chain.len(), 1);
    assert!(state_after_delta.total_delta_bytes > 0);
}

#[tokio::test]
async fn test_delta_size_threshold_under_5mb() {
    let dir = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(dir.path().join("db-size"))
            .build()
            .unwrap(),
    );
    let storage = SurrealStorage::new(tree).await.unwrap();

    // Establish baseline full snapshot first.
    storage.build_snapshot_auto(1).await.unwrap();

    for i in 1..=1100u64 {
        let req = KVRequest::Set {
            key: format!("s{}", i),
            value: vec![i as u8; 64],
        };
        storage.append_log_entry(1, i, &req).await.unwrap();
    }

    let snapshot = storage.build_snapshot_auto(1).await.unwrap();
    assert!(snapshot.meta.snapshot_id.starts_with("delta-"));

    let decoded = decode_snapshot_payload(snapshot.snapshot.get_ref())
        .unwrap()
        .unwrap();
    match decoded {
        DecodedSnapshotPayload::Delta { metadata, .. } => {
            assert!(metadata.size_bytes < DELTA_MAX_BYTES);
            assert_eq!(metadata.version, 2);
        }
        DecodedSnapshotPayload::Full(_) => panic!("expected delta payload"),
    }
}
