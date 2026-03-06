//! Unified error types for the Raft system.
//!
//! This module provides error handling compatible with both OpenRaft 0.9.21 and 0.10.0-alpha.15,
//! with feature flags determining which version's error types are imported.

use std::fmt;
use std::io;

/// Result type alias for Raft operations
pub type Result<T> = std::result::Result<T, RaftError>;

/// Error type alias for convenience
pub type Error = RaftError;

/// Comprehensive error type for Raft operations
#[derive(Debug)]
pub enum RaftError {
    /// IO error from SurrealKV or filesystem
    Io(io::Error),

    /// Serialization/deserialization error
    Serialization(String),

    /// Storage operation error
    Storage(String),

    /// Network communication error
    Network(String),

    /// Raft protocol violation
    RaftProto(String),

    /// Snapshot operation error
    Snapshot(String),

    /// State machine operation error
    StateMachine(String),

    /// Generic custom error
    Other(String),
}

impl fmt::Display for RaftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RaftError::Io(e) => write!(f, "IO error: {}", e),
            RaftError::Serialization(msg) => write!(f, "Serialization error: {}", msg),
            RaftError::Storage(msg) => write!(f, "Storage error: {}", msg),
            RaftError::Network(msg) => write!(f, "Network error: {}", msg),
            RaftError::RaftProto(msg) => write!(f, "Raft protocol error: {}", msg),
            RaftError::Snapshot(msg) => write!(f, "Snapshot error: {}", msg),
            RaftError::StateMachine(msg) => write!(f, "State machine error: {}", msg),
            RaftError::Other(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl std::error::Error for RaftError {}

impl From<io::Error> for RaftError {
    fn from(err: io::Error) -> Self {
        RaftError::Io(err)
    }
}

impl From<postcard::Error> for RaftError {
    fn from(err: postcard::Error) -> Self {
        RaftError::Serialization(err.to_string())
    }
}

impl From<anyhow::Error> for RaftError {
    fn from(err: anyhow::Error) -> Self {
        RaftError::Other(err.to_string())
    }
}

impl From<surrealkv::Error> for RaftError {
    fn from(err: surrealkv::Error) -> Self {
        RaftError::Storage(format!("SurrealKV error: {}", err))
    }
}
