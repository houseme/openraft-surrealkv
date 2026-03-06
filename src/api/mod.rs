//! HTTP REST API 模块（Phase 5）
//!
//! 提供用户友好的 HTTP API 访问 KV 存储：
//! - GET /kv/:key - 读取键值
//! - POST /kv/:key - 写入键值  
//! - DELETE /kv/:key - 删除键值
//! - GET /metrics - Prometheus 指标
//! - GET /health - 健康检查
//! - GET /status - Raft 集群状态

pub mod handlers;
pub mod middleware;
pub mod server;

pub use server::HttpServer;
