use crate::api::handlers::{
    delete_key, get_key, health_check, put_key, ready_check, status, AppState,
};
use crate::app::RaftNode;
use crate::metrics::render_metrics;
use crate::storage::SurrealStorage;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::info;

/// Wrap the Axum router and listening address for the HTTP API server.
///
/// Expose lightweight health/readiness endpoints and key-value CRUD routes
/// for local development and integration scenarios.
pub struct HttpServer {
    app: Router,
    addr: SocketAddr,
}

impl HttpServer {
    /// Create an HTTP server that serves storage directly (without a Raft handle).
    pub fn new(storage: Arc<SurrealStorage>, node_id: u64, port: u16) -> Self {
        let state = AppState {
            storage,
            raft_node: None,
            node_id,
        };
        Self::with_app_state(state, port)
    }

    /// Create an HTTP server backed by a `RaftNode` write path.
    pub fn with_raft(
        storage: Arc<SurrealStorage>,
        raft_node: Arc<RaftNode>,
        node_id: u64,
        port: u16,
    ) -> Self {
        let state = AppState {
            storage,
            raft_node: Some(raft_node),
            node_id,
        };
        Self::with_app_state(state, port)
    }

    /// Build the Axum router and apply common middleware (CORS and tracing).
    fn with_app_state(state: AppState, port: u16) -> Self {
        let app = Router::new()
            // Key-value routes.
            .route("/kv/:key", get(get_key))
            .route("/kv/:key", post(put_key))
            .route("/kv/:key", delete(delete_key))
            // Probe and status routes.
            .route("/health", get(health_check))
            .route("/ready", get(ready_check))
            .route("/status", get(status))
            // Metrics route (served via metrics-exporter-prometheus).
            .route("/metrics", get(metrics_handler))
            // Shared app state.
            .with_state(state)
            // Middleware: CORS.
            .layer(CorsLayer::permissive())
            // Middleware: request tracing.
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::default().include_headers(true)),
            );

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        Self { app, addr }
    }

    /// Serve requests on the configured address until shutdown or error.
    pub async fn serve(self) -> anyhow::Result<()> {
        info!("HTTP server listening on {}", self.addr);

        let listener = tokio::net::TcpListener::bind(self.addr).await?;

        axum::serve(listener, self.app)
            .await
            .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))?;

        Ok(())
    }
}

/// Serve `GET /metrics` by rendering Prometheus exposition text.
async fn metrics_handler() -> String {
    render_metrics()
}
