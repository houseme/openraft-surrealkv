//! OpenRaft + SurrealKV distributed consensus KV storage.
//!
//! # Overview
//!
//! Build a production-oriented distributed KV system by combining:
//! - **OpenRaft 0.10.0-alpha.15** (default) or **openraft-legacy 0.10.0-alpha.15** for consensus
//! - **SurrealKV 0.20+** as the LSM-based storage engine
//!
//! # Architecture
//!
//! Organize the system into modular layers:
//! - **Types** (`types.rs`): OpenRaft TypeConfig and KV operation types
//! - **Storage** (`storage.rs`): unified RaftLogStorage + RaftStateMachine implementation
//! - **State** (`state.rs`): persistent metadata management
//! - **Snapshot** (`snapshot.rs`): snapshot build/install pipeline
//! - **Network** (`network.rs`): tonic gRPC transport for Raft RPCs
//! - **Merge** (`merge.rs`): hybrid delta-merge strategy
//!
//! # Storage Layout
//!
//! All data is persisted in a single SurrealKV engine with organized key prefixes:
//! ```text
//! raft_log:{term}:{index}      → Log entries (postcard-serialized)
//! raft_vote                     → Voting state (immediate persistence)
//! raft_applied                  → Applied log position
//! raft_meta:*                   → Raft metadata
//! raft_snapshot_meta            → Snapshot chain state
//! app_data:*                    → Application state machine data
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use openraft_surrealkv::storage::SurrealStorage;
//! use openraft_surrealkv::types::{RaftTypeConfig, NodeId};
//! use std::sync::Arc;
//! use surrealkv::Engine;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Initialize SurrealKV engine
//!     let engine = Arc::new(Engine::new("./data").unwrap());
//!
//!     // Create unified storage
//!     let storage = SurrealStorage::new(engine).await.unwrap();
//!
//!     // Ready to use with OpenRaft
//! }
//! ```

pub mod error;
pub mod merge;
pub mod metrics;
pub mod network;
pub mod raft_adapter;
pub mod snapshot;
pub mod state;
pub mod storage;
mod storage_raft_impl;
pub mod types;

pub mod proto;

// HTTP and runtime-facing modules.
pub mod api;
pub mod app;
pub mod config;
pub mod shutdown;

// Re-export commonly used types.
pub use error::{RaftError, Result};
pub use raft_adapter::RaftConfig;
pub use types::{
    DeltaEntry, DeltaMetadata, KVRequest, KVResponse, NodeId, RaftTypeConfig, SnapshotData,
    SnapshotFormat,
};

// Re-export runtime entry points.
pub use app::RaftNode;
pub use config::Config;
pub use shutdown::ShutdownSignal;

/// Expose crate version from Cargo metadata.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
