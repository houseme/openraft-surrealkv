//! OpenRaft + SurrealKV Distributed Consensus KV Storage System
//!
//! # Overview
//!
//! This crate implements a production-grade distributed KV storage system combining:
//! - **OpenRaft 0.10.0-alpha.15** (default) or **openraft-legacy 0.10.0-alpha.15**: State-of-the-art Raft consensus
//! - **SurrealKV 0.20+**: High-performance LSM-based KV engine for all storage layers
//!
//! # Architecture
//!
//! The system is organized into modular layers:
//! - **Types** (`types.rs`): OpenRaft TypeConfig and KV operation types
//! - **Storage** (`storage.rs`): Unified RaftLogStorage + RaftStateMachine implementation
//! - **State** (`state.rs`): Persistent metadata management
//! - **Snapshot** (`snapshot.rs`): Snapshot building and installation
//! - **Network** (`network.rs`): Tonic gRPC network layer (Phase 1)
//! - **Merge** (`merge.rs`): Hybrid delta merge strategy (Phase 4)
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

// Phase 5: HTTP API 与配置管理
pub mod api;
pub mod config;

// Phase 5.2: Raft Node 与应用状态
pub mod app;
pub mod shutdown;

// Re-export commonly used types
pub use error::{RaftError, Result};
pub use raft_adapter::RaftConfig;
pub use types::{
    DeltaEntry, DeltaMetadata, KVRequest, KVResponse, NodeId, RaftTypeConfig, SnapshotData,
    SnapshotFormat,
};

// Phase 5 exports
pub use app::RaftNode;
pub use config::Config;
pub use shutdown::ShutdownSignal;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
