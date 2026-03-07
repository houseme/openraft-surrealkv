use openraft_surrealkv::api::HttpServer;
use openraft_surrealkv::app::RaftNode;
use openraft_surrealkv::config::Config;
use openraft_surrealkv::merge::DeltaMergePolicy;
use openraft_surrealkv::metrics::{init_prometheus_recorder, AppMetrics};
use openraft_surrealkv::network::server::{
    start_server_with_shutdown, ServerConfig as RaftServerConfig,
};
use openraft_surrealkv::network::GrpcRaftNetworkFactory;
use openraft_surrealkv::shutdown::ShutdownSignal;
use openraft_surrealkv::storage::SurrealStorage;
use std::collections::HashMap;
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. 加载配置
    let config = Config::load()?;

    // 2. 初始化日志系统
    init_logging(&config)?;

    // 2.1 启动前自检（端口/目录）
    config.preflight_check()?;

    // 2.2 初始化 Prometheus 指标导出器
    init_prometheus_recorder()?;
    let app_metrics = AppMetrics::new(config.node.node_id);
    app_metrics.raft.record_state("booting");
    app_metrics.raft.record_current_term(0);

    info!(
        "Starting OpenRaft-SurrealKV v{} (node_id={})",
        openraft_surrealkv::VERSION,
        config.node.node_id
    );

    // 3. 创建数据目录
    config.ensure_data_dir()?;

    // 4. 创建 SurrealKV Tree
    info!(
        "Initializing SurrealKV storage at {:?}",
        config.node.data_dir
    );
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(config.node.data_dir.join("kv"))
            .build()?,
    );

    // 5. 创建 Storage 并启用自动合并（Phase 4）
    let merge_policy = DeltaMergePolicy {
        max_chain_length: config.snapshot.max_delta_chain,
        max_delta_bytes: config.snapshot.max_delta_bytes_mb * 1024 * 1024,
        checkpoint_interval_secs: config.snapshot.checkpoint_interval_secs,
    };

    let storage = Arc::new(
        SurrealStorage::new(tree)
            .await?
            .with_merge_policy(merge_policy, config.node.node_id.to_string()),
    );

    // 6. 崩溃恢复（Phase 4）
    info!("Running merge state recovery check...");
    storage.recover_merge_state().await?;
    info!("Merge state recovery completed");

    // 7. Phase 5.2: 创建 Raft Node
    info!("Creating Raft node (Phase 5.2)...");
    let network_factory = Arc::new(GrpcRaftNetworkFactory::new(Arc::new(
        StaticAddressResolver::new(config.resolver_addresses()),
    )));

    let raft_node = match RaftNode::new(&config, storage.clone(), network_factory).await {
        Ok(node) => {
            info!("Raft node created successfully");
            app_metrics.raft.record_state("running");
            Arc::new(node)
        }
        Err(e) => {
            error!("Failed to create Raft node: {}", e);
            error!("Continuing without Raft (will use storage directly)");
            app_metrics.raft.record_state("standalone");
            // 继续运行，但 HTTP handlers 将使用存储直接读写
            Arc::new(RaftNode::new_standalone(&config, storage.clone()).await?)
        }
    };

    // 8. 创建优雅关闭信号
    let shutdown_signal = ShutdownSignal::new();

    if config.cluster.bootstrap && raft_node.is_real_raft() {
        let members = config.cluster_members();
        info!(
            node_id = config.node.node_id,
            members = members.len(),
            "Bootstrapping raft cluster membership"
        );

        if let Err(e) = raft_node.initialize_cluster(members).await {
            tracing::warn!(error = %e, "raft bootstrap initialize skipped");
        }
    }

    // 8.1 启动 Raft gRPC 服务（仅真实 Raft 模式）
    let mut raft_server_handle = None;
    let mut raft_shutdown_tx = None;

    if let Some(raft_handle) = raft_node.raft_handle() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        raft_shutdown_tx = Some(tx);

        let raft_addr = config.node.listen_addr.clone();
        info!(addr = %raft_addr, "Starting Raft gRPC server");

        raft_server_handle = Some(tokio::spawn(async move {
            let cfg = RaftServerConfig {
                addr: raft_addr,
                ..Default::default()
            };
            if let Err(e) = start_server_with_shutdown(cfg, Arc::new(raft_handle), async move {
                let _ = rx.await;
            })
            .await
            {
                error!(error = %e, "Raft gRPC server failed");
            }
        }));
    } else {
        info!("Raft gRPC server disabled (standalone mode)");
    }

    // 9. 启动 HTTP 服务器（如果启用）
    let mut http_server_handle = None;
    if config.http.enabled {
        let http_server = HttpServer::with_raft(
            storage.clone(),
            raft_node.clone(),
            config.node.node_id,
            config.http.port,
        );

        let addr = format!("0.0.0.0:{}", config.http.port);
        info!(addr = %addr, "Starting HTTP API server");

        http_server_handle = Some(tokio::spawn(async move {
            if let Err(e) = http_server.serve().await {
                error!(error = %e, "HTTP server failed");
            }
        }));
    }

    // 启动自检日志
    let ready_probe = storage.read("__ready_probe__").await;
    let ready_details = match ready_probe {
        Ok(_) => "storage_ok latency=0ms".to_string(),
        Err(e) => format!("storage_error: {}", e),
    };
    let raft_status = if raft_node.is_standalone {
        "standalone".to_string()
    } else {
        raft_node.current_role().await
    };

    info!(
        node_id = config.node.node_id,
        raft_status = %raft_status,
        ready_details = %ready_details,
        applied_index = raft_node.applied_index().await,
        "🚀 Node startup self-check complete"
    );

    info!("Waiting for Ctrl+C");
    tokio::signal::ctrl_c().await?;
    shutdown_signal.shutdown();
    info!("Shutdown signal received, stopping services");

    if let Some(tx) = raft_shutdown_tx {
        let _ = tx.send(());
    }

    if let Some(handle) = http_server_handle {
        info!("Stopping HTTP server task");
        handle.abort();
        let _ = handle.await;
    }

    if let Some(handle) = raft_server_handle {
        info!("Waiting for Raft gRPC server to stop");
        let _ = handle.await;
    }

    if let Err(e) = raft_node.shutdown().await {
        error!(error = %e, "Raft shutdown failed");
    }

    info!("Shutting down gracefully...");
    // TODO: 优雅关闭逻辑

    Ok(())
}

/// 初始化日志系统
fn init_logging(config: &Config) -> anyhow::Result<()> {
    let env_filter =
        EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(&config.logging.level))?;

    let subscriber = tracing_subscriber::registry().with(env_filter);

    match config.logging.format.as_str() {
        "json" => {
            let fmt_layer = fmt::layer().json();
            subscriber.with(fmt_layer).init();
        }
        _ => {
            let fmt_layer = fmt::layer();
            subscriber.with(fmt_layer).init();
        }
    }

    Ok(())
}

/// 静态地址解析器（来自配置）
struct StaticAddressResolver {
    addresses: HashMap<u64, String>,
}

impl StaticAddressResolver {
    fn new(addresses: HashMap<u64, String>) -> Self {
        Self { addresses }
    }
}

#[async_trait::async_trait]
impl openraft_surrealkv::network::client::AddressResolver for StaticAddressResolver {
    async fn resolve(
        &self,
        node_id: u64,
    ) -> anyhow::Result<String, openraft_surrealkv::error::Error> {
        self.addresses.get(&node_id).cloned().ok_or_else(|| {
            openraft_surrealkv::error::Error::Network(format!(
                "no address found for node {}",
                node_id
            ))
        })
    }
}
