use crate::app::RaftNode;
use crate::storage::SurrealStorage;
use crate::types::KVRequest;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 应用状态（共享给所有 HTTP 处理器）
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<SurrealStorage>,
    pub raft_node: Option<Arc<RaftNode>>, // Phase 5.2: Raft Node (可选，如果启用)
    pub node_id: u64,
}

/// API 错误响应
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// API 成功响应
#[derive(Debug, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub message: String,
}

/// KV 值响应
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueResponse {
    pub value: String, // Base64 编码的值
}

/// 健康检查响应
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub node_id: u64,
}

/// 就绪详情
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadyDetails {
    pub probe_latency_ms: u64,
    pub last_error: Option<String>,
    pub checked_at_unix_ms: u64,
}

/// 就绪检查响应
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub mode: String,
    pub node_id: u64,
    pub details: ReadyDetails,
}

/// 状态响应
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub node_id: u64,
    pub role: String,
    pub term: u64,
    pub applied_index: u64,
}

/// GET /kv/:key - 读取键值
pub async fn get_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<ValueResponse>, AppError> {
    let value = state
        .storage
        .read(&key)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    match value {
        Some(v) => Ok(Json(ValueResponse {
            value: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, v),
        })),
        None => Err(AppError::NotFound(format!("Key '{}' not found", key))),
    }
}

/// POST /kv/:key - 写入键值（Phase 5.2: 通过 RaftNode client_write 路径）
pub async fn put_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<SuccessResponse>, AppError> {
    let req = KVRequest::Set {
        key: key.clone(),
        value: body.to_vec(),
    };

    if let Some(raft_node) = &state.raft_node {
        raft_node
            .client_write(req)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    } else {
        state
            .storage
            .apply_request(&req)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    Ok(Json(SuccessResponse {
        message: format!("Key '{}' updated successfully", key),
    }))
}

/// DELETE /kv/:key - 删除键值（Phase 5.2: 通过 RaftNode client_write 路径）
pub async fn delete_key(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<SuccessResponse>, AppError> {
    let req = KVRequest::Delete { key: key.clone() };

    if let Some(raft_node) = &state.raft_node {
        raft_node
            .client_write(req)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    } else {
        state
            .storage
            .apply_request(&req)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    Ok(Json(SuccessResponse {
        message: format!("Key '{}' deleted successfully", key),
    }))
}

/// GET /health - 健康检查
pub async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        node_id: state.node_id,
    })
}

/// GET /ready - 就绪检查
pub async fn ready_check(State(state): State<AppState>) -> Json<ReadyResponse> {
    // Option A: 仅进程 + 存储可用即 ready。
    let started = std::time::Instant::now();
    let storage_probe = state.storage.read("__ready_probe__").await;
    let latency = started.elapsed().as_millis() as u64;
    let checked_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let (ready, last_error) = match storage_probe {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };

    Json(ReadyResponse {
        ready,
        mode: "process+storage".to_string(),
        node_id: state.node_id,
        details: ReadyDetails {
            probe_latency_ms: latency,
            last_error,
            checked_at_unix_ms,
        },
    })
}

/// GET /status - Raft 集群状态
pub async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let applied = state.storage.metadata().get_applied_state().await;
    let voting = state.storage.metadata().get_voting_state().await;

    // Phase 5.2: 从 Raft Node 获取实际角色
    let (role, term) = if let Some(raft_node) = &state.raft_node {
        let role = raft_node.current_role().await;
        let term = raft_node.current_term().await;
        (role, term)
    } else {
        ("unknown".to_string(), voting.current_term)
    };

    Json(StatusResponse {
        node_id: state.node_id,
        role,
        term,
        applied_index: applied.last_applied_index,
    })
}

/// 应用错误类型
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_serialization() {
        let err = ErrorResponse {
            error: "test error".to_string(),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("test error"));
    }
}
