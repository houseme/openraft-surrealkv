use crate::app::RaftNode;
use crate::storage::SurrealStorage;
use crate::types::KVRequest;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Application state shared across HTTP handlers.
///
/// This struct is intentionally lightweight and contains:
/// - `storage`: SurrealStorage instance used for reads/writes
/// - `raft_node`: optional RaftNode handle (present when cluster features are enabled)
/// - `node_id`: numeric ID for the running node (useful for health/debug endpoints)
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<SurrealStorage>,
    pub raft_node: Option<Arc<RaftNode>>, // Phase 5.2: Raft Node (optional when enabled)
    pub node_id: u64,
}

/// API error response body returned to clients.
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Simple success envelope for write operations.
#[derive(Debug, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub message: String,
}

/// Value response for GET /kv/:key. Encodes binary payload as Base64.
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueResponse {
    /// Base64-encoded value bytes
    pub value: String,
}

/// Health probe response (synthetic quick check).
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub node_id: u64,
}

/// Detailed readiness information returned by GET /ready.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadyDetails {
    /// Measured probe latency in milliseconds
    pub probe_latency_ms: u64,
    /// Last observed error from the probe, if any
    pub last_error: Option<String>,
    /// Timestamp when the ready check ran (unix ms)
    pub checked_at_unix_ms: u64,
}

/// Readiness envelope.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadyResponse {
    pub ready: bool,
    /// Human-readable readiness mode, e.g. "process+storage"
    pub mode: String,
    pub node_id: u64,
    pub details: ReadyDetails,
}

/// Node/cluster status returned by GET /status.
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub node_id: u64,
    /// Role as string: Leader, Follower, Candidate, Standalone, etc.
    pub role: String,
    pub term: u64,
    pub applied_index: u64,
}

/// GET /kv/:key - read a key value from the local state machine
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

/// POST /kv/:key - write a key value (Phase 5.2: through RaftNode client_write when enabled)
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

/// DELETE /kv/:key - delete a key (Phase 5.2: through RaftNode client_write when enabled)
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

/// GET /health - lightweight health probe
pub async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        node_id: state.node_id,
    })
}

/// GET /ready - readiness probe
pub async fn ready_check(State(state): State<AppState>) -> Json<ReadyResponse> {
    // Option A: ready when process is up and storage is accessible.
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

/// GET /status - Raft cluster/node status
pub async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let applied = state.storage.metadata().get_applied_state().await;
    let voting = state.storage.metadata().get_voting_state().await;

    // Phase 5.2: obtain role and term from RaftNode when available
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

/// Application-level error types returned by handlers
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
