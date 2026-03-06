//! Core OpenRaft type definitions and configuration.

use openraft::raft::responder::Responder;
use openraft::{BasicNode, LogId};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

/// NodeId type: 64-bit unsigned integer
pub type NodeId = u64;

/// SnapshotData type: Cursor wrapper for AsyncRead/AsyncSeek support
pub type SnapshotData = Cursor<Vec<u8>>;

/// KV request types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum KVRequest {
    Set { key: String, value: Vec<u8> },
    Delete { key: String },
}

impl std::fmt::Display for KVRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KVRequest::Set { key, value } => {
                write!(f, "Set {{ key: \"{}\", value: {:?} }}", key, value)
            }
            KVRequest::Delete { key } => write!(f, "Delete {{ key: \"{}\" }}", key),
        }
    }
}

/// KV response types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum KVResponse {
    Ok,
    Value(Vec<u8>),
    Err(String),
}

impl std::fmt::Display for KVResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KVResponse::Ok => write!(f, "Ok"),
            KVResponse::Value(_) => write!(f, "Value"),
            KVResponse::Err(e) => write!(f, "Err: {}", e),
        }
    }
}

/// Phase 0 Responder - Simple no-op implementation for now
#[derive(Debug, Clone)]
pub struct Phase0Responder<T>(std::marker::PhantomData<T>);

impl<T: Send + 'static> Responder<RaftTypeConfig, T> for Phase0Responder<T> {
    fn on_commit(&mut self, _log_id: LogId<RaftTypeConfig>) {
        // TODO: Implement in Phase 2
    }
    fn on_complete(self, _result: T) {
        // TODO: Implement in Phase 2
    }
}

/// OpenRaft TypeConfig implementation for SurrealKV
///
/// This configuration uses:
/// - KVRequest as the application request type
/// - KVResponse as the application response type
/// - u64 for node IDs
/// - BasicNode for cluster member information
/// - Cursor<Vec<u8>> for snapshot streaming
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct RaftTypeConfig;

impl openraft::RaftTypeConfig for RaftTypeConfig {
    type D = KVRequest;
    type R = KVResponse;
    type NodeId = NodeId;
    type Node = BasicNode;
    type Term = u64;
    type LeaderId = openraft::impls::leader_id_std::LeaderId<Self>;
    type Vote = openraft::Vote<Self>;
    type Entry = openraft::Entry<Self>;
    type SnapshotData = SnapshotData;
    type AsyncRuntime = openraft::TokioRuntime;
    type Responder<T: Send + 'static> = Phase0Responder<T>;
    type ErrorSource = openraft::AnyError;
}

/// Delta entry format stored in incremental snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeltaEntry {
    pub term: u64,
    pub index: u64,
    /// Postcard-encoded `KVRequest` payload.
    pub payload: Vec<u8>,
}

impl DeltaEntry {
    pub fn new(term: u64, index: u64, payload: Vec<u8>) -> Self {
        Self {
            term,
            index,
            payload,
        }
    }
}

/// Snapshot payload format used by Phase 3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SnapshotFormat {
    /// Full checkpoint snapshot (tar.zst bytes).
    Full {
        checkpoint_meta: crate::snapshot::CheckpointMetadata,
    },
    /// Delta snapshot metadata only (entries are stored in payload bytes).
    Delta { base_index: u64, entry_count: u64 },
}

/// Delta metadata persisted in snapshot state for policy decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaMetadata {
    pub format: SnapshotFormat,
    pub size_bytes: u64,
    pub created_at: u64,
    pub file_path: Option<String>,
    pub version: u16,
}

impl DeltaMetadata {
    pub fn new(format: SnapshotFormat, size_bytes: u64, created_at: u64) -> Self {
        Self {
            format,
            size_bytes,
            created_at,
            file_path: None,
            version: 2,
        }
    }

    pub fn with_file_path(mut self, path: String) -> Self {
        self.file_path = Some(path);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kv_request_set() {
        let req = KVRequest::Set {
            key: "test".to_string(),
            value: vec![1, 2, 3],
        };
        assert_eq!(req.to_string(), "Set { key: \"test\", value: [1, 2, 3] }");
    }

    #[test]
    fn test_kv_response_ok() {
        let resp = KVResponse::Ok;
        assert_eq!(resp.to_string(), "Ok");
    }

    #[test]
    fn test_kv_response_err() {
        let resp = KVResponse::Err("test error".to_string());
        assert_eq!(resp.to_string(), "Err: test error");
    }

    #[test]
    fn test_delta_entry_creation() {
        let e = DeltaEntry::new(2, 9, vec![1, 2, 3]);
        assert_eq!(e.term, 2);
        assert_eq!(e.index, 9);
        assert_eq!(e.payload, vec![1, 2, 3]);
    }

    #[test]
    fn test_delta_metadata_builder() {
        let meta = DeltaMetadata::new(
            SnapshotFormat::Delta {
                base_index: 10,
                entry_count: 1,
            },
            1024,
            123,
        )
        .with_file_path("/tmp/delta.zst".to_string());

        assert_eq!(meta.size_bytes, 1024);
        assert_eq!(meta.version, 2);
        assert_eq!(meta.file_path.as_deref(), Some("/tmp/delta.zst"));
    }
}
