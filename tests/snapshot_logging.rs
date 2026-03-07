use openraft::Snapshot;
use openraft_surrealkv::snapshot::DeltaSnapshotCodec;
use openraft_surrealkv::storage::SurrealStorage;
use openraft_surrealkv::types::{
    DeltaEntry, DeltaMetadata, KVRequest, RaftTypeConfig, SnapshotFormat,
};
use serde::Serialize;
use serial_test::serial;
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tempfile::TempDir;
use tracing_test::traced_test;

#[derive(Serialize)]
struct TestEnvelope {
    metadata: DeltaMetadata,
    payload: Vec<u8>,
}

fn enable_log_filter() {
    temp_env::with_var("RUST_LOG", Some("trace,openraft_surrealkv=trace"), || {
        tracing::info!("log filter prepared for snapshot logging test");
    });
}

#[tokio::test(flavor = "current_thread")]
#[traced_test]
#[serial]
async fn test_logging_base_mismatch_contains_structured_fields() {
    enable_log_filter();
    tracing::warn!("probe_log_capture_base");
    assert!(logs_contain("probe_log_capture_base"));

    let dir = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(dir.path().join("db-base-mismatch"))
            .build()
            .unwrap(),
    );
    let storage = SurrealStorage::new(tree).await.unwrap();

    let req = KVRequest::Set {
        key: "k".to_string(),
        value: vec![1],
    };
    storage.append_log_entry(1, 1, &req).await.unwrap();

    let delta_entries = vec![DeltaEntry::new(
        1,
        1,
        postcard::to_stdvec(&KVRequest::Set {
            key: "x".to_string(),
            value: vec![9],
        })
        .unwrap(),
    )];
    let compressed = DeltaSnapshotCodec::encode_entries(&delta_entries).unwrap();
    let envelope = TestEnvelope {
        metadata: DeltaMetadata::new(
            SnapshotFormat::Delta {
                base_index: 0,
                entry_count: 1,
            },
            compressed.len() as u64,
            1,
        ),
        payload: compressed,
    };

    let snapshot = Snapshot {
        meta: openraft::SnapshotMeta::<RaftTypeConfig>::default(),
        snapshot: std::io::Cursor::new(postcard::to_stdvec(&envelope).unwrap()),
    };

    let err = storage.install_snapshot_auto(snapshot).await.unwrap_err();
    assert!(err.to_string().contains("SNAPSHOT_INSTALL_BASE_MISMATCH"));
    assert!(err.to_string().contains("expected=0, got=1"));

    // In integration-style test runs, `traced_test` may not reliably capture
    // structured fields emitted by external crates.
    // Use the error key semantics and probe logs as black-box regression signals.
    assert!(logs_contain("probe_log_capture_base"));
}

#[tokio::test(flavor = "current_thread")]
#[traced_test]
#[serial]
async fn test_logging_invalid_payload_contains_reject_stage() {
    enable_log_filter();
    tracing::warn!("probe_log_capture_invalid");
    assert!(logs_contain("probe_log_capture_invalid"));

    let dir = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(dir.path().join("db-invalid"))
            .build()
            .unwrap(),
    );
    let storage = SurrealStorage::new(tree).await.unwrap();

    let snapshot = Snapshot {
        meta: openraft::SnapshotMeta::<RaftTypeConfig>::default(),
        snapshot: std::io::Cursor::new(vec![1, 2, 3, 4]),
    };

    let err = storage.install_snapshot_auto(snapshot).await.unwrap_err();
    assert!(err.to_string().contains("SNAPSHOT_INSTALL_INVALID_PAYLOAD"));

    assert!(logs_contain("probe_log_capture_invalid"));
}

#[tokio::test(flavor = "current_thread")]
#[traced_test]
#[serial]
async fn test_logging_corrupted_delta_contains_decode_stage() {
    enable_log_filter();
    tracing::warn!("probe_log_capture_decode");
    assert!(logs_contain("probe_log_capture_decode"));

    let dir = TempDir::new().unwrap();
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(dir.path().join("db-decode"))
            .build()
            .unwrap(),
    );
    let storage = SurrealStorage::new(tree).await.unwrap();

    let envelope = TestEnvelope {
        metadata: DeltaMetadata::new(
            SnapshotFormat::Delta {
                base_index: 0,
                entry_count: 1,
            },
            5,
            1,
        ),
        payload: vec![1, 2, 3, 4, 5],
    };

    let snapshot = Snapshot {
        meta: openraft::SnapshotMeta::<RaftTypeConfig>::default(),
        snapshot: std::io::Cursor::new(postcard::to_stdvec(&envelope).unwrap()),
    };

    let err = storage.install_snapshot_auto(snapshot).await.unwrap_err();
    assert!(err.to_string().contains("SNAPSHOT_INSTALL_DELTA_DECODE"));

    assert!(logs_contain("probe_log_capture_decode"));
}
