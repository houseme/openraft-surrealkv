//! HTTP REST API module (Phase 5)
//!
//! Provides a lightweight HTTP interface for interacting with the KV storage and operational
//! endpoints useful for debugging and SRE integration. Exposed endpoints include:
//! - GET /kv/:key     -> read a key
//! - POST /kv/:key    -> write a key
//! - DELETE /kv/:key  -> delete a key
//! - GET /metrics     -> Prometheus metrics exposition
//! - GET /health      -> health probe
//! - GET /status      -> Raft cluster/node status

pub mod handlers;
pub mod middleware;
pub mod server;

pub use server::HttpServer;
