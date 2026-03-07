use openraft_surrealkv::app::RaftNode;
use openraft_surrealkv::config::{ClusterConfig, Config, PeerConfig};
use openraft_surrealkv::merge::DeltaMergePolicy;
use openraft_surrealkv::network::GrpcRaftNetworkFactory;
use openraft_surrealkv::network::client::AddressResolver;
use openraft_surrealkv::network::server::{ServerConfig, start_server_with_shutdown};
use openraft_surrealkv::storage::SurrealStorage;
use std::collections::{HashMap, HashSet};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;
use surrealkv::TreeBuilder;
use tempfile::TempDir;

#[derive(Clone)]
struct TestResolver {
    addrs: HashMap<u64, String>,
}

impl TestResolver {
    fn new(addrs: HashMap<u64, String>) -> Self {
        Self { addrs }
    }
}

#[async_trait::async_trait]
impl AddressResolver for TestResolver {
    async fn resolve(&self, node_id: u64) -> openraft_surrealkv::error::Result<String> {
        self.addrs.get(&node_id).cloned().ok_or_else(|| {
            openraft_surrealkv::error::Error::Network(format!("node {} not found", node_id))
        })
    }
}

pub type NodeStatus = (u64, String, u64, u64);

pub struct NodeRuntime {
    pub node: Arc<RaftNode>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<()>,
}

fn make_config(
    node_id: u64,
    listen_addr: &str,
    peers: Vec<PeerConfig>,
    data_dir: std::path::PathBuf,
) -> Config {
    let mut cfg = Config::default();
    cfg.node.node_id = node_id;
    cfg.node.listen_addr = listen_addr.to_string();
    cfg.node.data_dir = data_dir;
    cfg.http.enabled = false;
    cfg.cluster = ClusterConfig {
        bootstrap: node_id == 1,
        expected_voters: Some(3),
        peers,
    };
    cfg
}

async fn build_node(cfg: &Config, addrs: HashMap<u64, String>) -> anyhow::Result<NodeRuntime> {
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(cfg.node.data_dir.join("kv"))
            .build()?,
    );

    let storage = Arc::new(SurrealStorage::new(tree).await?.with_merge_policy(
        DeltaMergePolicy {
            max_chain_length: cfg.snapshot.max_delta_chain,
            max_delta_bytes: cfg.snapshot.max_delta_bytes_mb * 1024 * 1024,
            checkpoint_interval_secs: cfg.snapshot.checkpoint_interval_secs,
        },
        cfg.node.node_id.to_string(),
    ));

    let network_factory = Arc::new(GrpcRaftNetworkFactory::new(Arc::new(TestResolver::new(
        addrs,
    ))));
    let node = Arc::new(RaftNode::new(cfg, storage, network_factory).await?);

    let raft = node
        .raft_handle()
        .ok_or_else(|| anyhow::anyhow!("raft handle missing"))?;

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let addr = cfg.node.listen_addr.clone();
    let join = tokio::spawn(async move {
        let _ = start_server_with_shutdown(
            ServerConfig {
                addr,
                ..Default::default()
            },
            Arc::new(raft),
            async move {
                let _ = rx.await;
            },
        )
        .await;
    });

    Ok(NodeRuntime {
        node,
        shutdown_tx: Some(tx),
        join,
    })
}

fn allocate_dynamic_addrs(node_ids: &[u64]) -> anyhow::Result<HashMap<u64, String>> {
    let mut listeners = Vec::new();
    let mut addrs = HashMap::new();
    let mut seen = HashSet::new();

    for id in node_ids {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?.to_string();
        if !seen.insert(addr.clone()) {
            anyhow::bail!("duplicate ephemeral addr allocated: {}", addr);
        }
        addrs.insert(*id, addr);
        listeners.push(listener);
    }

    drop(listeners);
    Ok(addrs)
}

fn leader_count(roles: &[String]) -> usize {
    roles.iter().filter(|r| r.as_str() == "Leader").count()
}

fn leader_id_from_roles(roles: &[String]) -> Option<u64> {
    roles
        .iter()
        .position(|r| r == "Leader")
        .map(|i| (i + 1) as u64)
}

pub async fn snapshot_status(nodes: &[Arc<RaftNode>]) -> Vec<NodeStatus> {
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        out.push((
            n.node_id,
            n.current_role().await,
            n.current_term().await,
            n.applied_index().await,
        ));
    }
    out
}

pub fn format_status(status: &[NodeStatus]) -> String {
    status
        .iter()
        .map(|(id, role, term, applied)| {
            format!(
                "node={} role={} term={} applied_index={}",
                id, role, term, applied
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn nodes_of(runtimes: &[NodeRuntime]) -> Vec<Arc<RaftNode>> {
    runtimes.iter().map(|r| r.node.clone()).collect()
}

pub async fn wait_stable_single_leader(
    nodes: &[Arc<RaftNode>],
    timeout: Duration,
) -> anyhow::Result<(u64, Vec<NodeStatus>)> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut stable_single_leader_rounds = 0usize;

    while tokio::time::Instant::now() < deadline {
        let roles = vec![
            nodes[0].current_role().await,
            nodes[1].current_role().await,
            nodes[2].current_role().await,
        ];

        if leader_count(&roles) == 1 {
            stable_single_leader_rounds += 1;
            if stable_single_leader_rounds >= 3 {
                let leader_id = leader_id_from_roles(&roles)
                    .ok_or_else(|| anyhow::anyhow!("leader not found after stable election"))?;
                let status = snapshot_status(nodes).await;
                return Ok((leader_id, status));
            }
        } else {
            stable_single_leader_rounds = 0;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let status = snapshot_status(nodes).await;
    Err(anyhow::anyhow!(
        "expected exactly one stable Leader within timeout; status: {}",
        format_status(&status)
    ))
}

pub async fn start_three_nodes() -> anyhow::Result<(Vec<NodeRuntime>, Config)> {
    let d1 = TempDir::new()?;
    let d2 = TempDir::new()?;
    let d3 = TempDir::new()?;

    let addrs = allocate_dynamic_addrs(&[1, 2, 3])?;

    let peers_for = |self_id: u64| -> Vec<PeerConfig> {
        addrs
            .iter()
            .filter(|(id, _)| **id != self_id)
            .map(|(id, addr)| PeerConfig {
                node_id: *id,
                addr: addr.clone(),
            })
            .collect()
    };

    let c1 = make_config(
        1,
        addrs
            .get(&1)
            .ok_or_else(|| anyhow::anyhow!("missing addr for node 1"))?,
        peers_for(1),
        d1.path().to_path_buf(),
    );
    let c2 = make_config(
        2,
        addrs
            .get(&2)
            .ok_or_else(|| anyhow::anyhow!("missing addr for node 2"))?,
        peers_for(2),
        d2.path().to_path_buf(),
    );
    let c3 = make_config(
        3,
        addrs
            .get(&3)
            .ok_or_else(|| anyhow::anyhow!("missing addr for node 3"))?,
        peers_for(3),
        d3.path().to_path_buf(),
    );

    let n1 = build_node(&c1, addrs.clone()).await?;
    let n2 = build_node(&c2, addrs.clone()).await?;
    let n3 = build_node(&c3, addrs.clone()).await?;

    Ok((vec![n1, n2, n3], c1))
}

pub async fn shutdown_all(mut runtimes: Vec<NodeRuntime>) {
    for r in &mut runtimes {
        if let Some(tx) = r.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    for r in runtimes {
        let _ = r.join.await;
        let _ = r.node.shutdown().await;
    }
}
