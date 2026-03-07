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

/// HTTP server wrapper for the application's REST API.
///
/// This struct holds the Axum router and the listening address. The server exposes
/// lightweight health/readiness endpoints and a simple KV CRUD API used for local
/// testing and integration scenarios.
pub struct HttpServer {
    app: Router,
    addr: SocketAddr,
}

impl HttpServer {
    /// Create a new HTTP server without a Raft node (Phase 5.1).
    pub fn new(storage: Arc<SurrealStorage>, node_id: u64, port: u16) -> Self {
        let state = AppState {
            storage,
            raft_node: None,
            node_id,
        };
        Self::with_app_state(state, port)
    }

    /// Create an HTTP server backed by a RaftNode (Phase 5.2).
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

    /// Build the Axum Router and apply common middleware (CORS, tracing).
    fn with_app_state(state: AppState, port: u16) -> Self {
        let app = Router::new()
            // KV API
            .route("/kv/:key", get(get_key))
            .route("/kv/:key", post(put_key))
            .route("/kv/:key", delete(delete_key))
            // Health check
            .route("/health", get(health_check))
            // Readiness check
            .route("/ready", get(ready_check))
            // Cluster status
            .route("/status", get(status))
            // Metrics (provided by metrics-exporter-prometheus)
            .route("/metrics", get(metrics_handler))
            // State sharing
            .with_state(state)
            // Middleware: CORS
            .layer(CorsLayer::permissive())
            // Middleware: Request tracing
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::default().include_headers(true)),
            );

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        Self { app, addr }
    }

    /// Start serving requests on the configured address. This method blocks until
    /// the server is shut down or an error occurs.
    pub async fn serve(self) -> anyhow::Result<()> {
        info!("HTTP server listening on {}", self.addr);

        let listener = tokio::net::TcpListener::bind(self.addr).await?;

        axum::serve(listener, self.app)
            .await
            .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))?;

        Ok(())
    }
}

/// GET /metrics - Prometheus metrics rendering endpoint
async fn metrics_handler() -> String {
    render_metrics()
}
