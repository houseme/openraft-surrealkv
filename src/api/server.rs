use crate::api::handlers::{
    AppState, delete_key, get_key, health_check, put_key, ready_check, status,
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

/// HTTP 服务器
pub struct HttpServer {
    app: Router,
    addr: SocketAddr,
}

impl HttpServer {
    /// 创建新的 HTTP 服务器（Phase 5.1：无 Raft Node）
    pub fn new(storage: Arc<SurrealStorage>, node_id: u64, port: u16) -> Self {
        let state = AppState {
            storage,
            raft_node: None,
            node_id,
        };
        Self::with_app_state(state, port)
    }

    /// 创建带 Raft Node 的 HTTP 服务器（Phase 5.2）
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

    /// 使用 AppState 创建服务器
    fn with_app_state(state: AppState, port: u16) -> Self {
        let app = Router::new()
            // KV API
            .route("/kv/:key", get(get_key))
            .route("/kv/:key", post(put_key))
            .route("/kv/:key", delete(delete_key))
            // 健康检查
            .route("/health", get(health_check))
            // 就绪检查
            .route("/ready", get(ready_check))
            // 集群状态
            .route("/status", get(status))
            // Metrics（由 metrics-exporter-prometheus 提供）
            .route("/metrics", get(metrics_handler))
            // 状态共享
            .with_state(state)
            // 中间件：CORS
            .layer(CorsLayer::permissive())
            // 中间件：请求追踪
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::default().include_headers(true)),
            );

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        Self { app, addr }
    }

    /// 启动 HTTP 服务器
    pub async fn serve(self) -> anyhow::Result<()> {
        info!("HTTP server listening on {}", self.addr);

        let listener = tokio::net::TcpListener::bind(self.addr).await?;

        axum::serve(listener, self.app)
            .await
            .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))?;

        Ok(())
    }
}

/// GET /metrics - Prometheus 指标导出
async fn metrics_handler() -> String {
    render_metrics()
}
