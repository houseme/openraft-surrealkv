use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

/// OpenRaft-SurrealKV 分布式 KV 服务配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub node: NodeConfig,
    pub http: HttpConfig,
    pub raft: RaftConfig,
    #[serde(default)]
    pub cluster: ClusterConfig,
    pub snapshot: SnapshotConfig,
    pub storage: StorageConfig,
    pub logging: LoggingConfig,
    pub metrics: MetricsConfig,
}

/// 节点配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// 节点 ID（1, 2, 3, ...）
    pub node_id: u64,
    /// gRPC 监听地址
    pub listen_addr: String,
    /// 数据目录
    pub data_dir: PathBuf,
}

/// HTTP API 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// 是否启用 HTTP API
    pub enabled: bool,
    /// HTTP 监听端口
    pub port: u16,
}

/// Raft 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftConfig {
    /// 心跳间隔（毫秒）
    pub heartbeat_interval_ms: u64,
    /// 选举超时（毫秒）
    pub election_timeout_ms: u64,
    /// 最大批量日志条目数
    pub max_payload_entries: u64,
}

/// 快照配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotConfig {
    /// Checkpoint 间隔（秒）
    pub checkpoint_interval_secs: u64,
    /// Delta 链最大长度
    pub max_delta_chain: usize,
    /// Delta 累计最大字节数（MB）
    pub max_delta_bytes_mb: u64,
}

/// 存储配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// 是否启用压缩
    pub enable_compression: bool,
    /// 刷新间隔（毫秒）
    pub flush_interval_ms: u64,
}

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// 日志级别（trace/debug/info/warn/error）
    pub level: String,
    /// 日志格式（json/text）
    pub format: String,
}

/// 指标配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// 是否启用指标导出
    pub enabled: bool,
    /// 指标监听地址
    pub listen_addr: String,
}

/// 集群配置（多节点 Bootstrap/Join）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterConfig {
    /// 当前节点是否执行 bootstrap 初始化
    pub bootstrap: bool,
    /// 期望 voter 数量（self + peers），用于防误配置
    pub expected_voters: Option<usize>,
    /// 已知集群节点（不包含本节点）
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

/// 对端节点配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    pub node_id: u64,
    pub addr: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            node: NodeConfig {
                node_id: 1,
                listen_addr: "127.0.0.1:50051".to_string(),
                data_dir: PathBuf::from("./data/node_1"),
            },
            http: HttpConfig {
                enabled: true,
                port: 8080,
            },
            raft: RaftConfig {
                heartbeat_interval_ms: 500,
                election_timeout_ms: 3000,
                max_payload_entries: 300,
            },
            cluster: ClusterConfig::default(),
            snapshot: SnapshotConfig {
                checkpoint_interval_secs: 3600,
                max_delta_chain: 5,
                max_delta_bytes_mb: 300,
            },
            storage: StorageConfig {
                enable_compression: true,
                flush_interval_ms: 1000,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "text".to_string(),
            },
            metrics: MetricsConfig {
                enabled: true,
                listen_addr: "0.0.0.0:9090".to_string(),
            },
        }
    }
}

/// 命令行参数
#[derive(Debug, Clone, Parser)]
#[command(name = "openraft-surrealkv")]
#[command(about = "OpenRaft + SurrealKV 分布式 KV 服务", long_about = None)]
pub struct CliArgs {
    /// 配置文件路径
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// 节点 ID（覆盖配置文件）
    #[arg(long, env = "NODE_ID")]
    pub node_id: Option<u64>,

    /// 监听地址（覆盖配置文件）
    #[arg(long, env = "LISTEN_ADDR")]
    pub listen_addr: Option<String>,

    /// 数据目录（覆盖配置文件）
    #[arg(long, env = "DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// HTTP 端口（覆盖配置文件）
    #[arg(long, env = "HTTP_PORT")]
    pub http_port: Option<u16>,

    /// 日志级别（覆盖配置文件）
    #[arg(long, env = "LOG_LEVEL")]
    pub log_level: Option<String>,
}

impl Config {
    /// 从命令行参数和配置文件加载配置
    ///
    /// 优先级：命令行参数 > 环境变量 > 配置文件 > 默认值
    pub fn load() -> anyhow::Result<Self> {
        let args = CliArgs::parse();

        // 1. 加载配置文件或使用默认配置
        let mut config = if let Some(config_path) = &args.config {
            let content = std::fs::read_to_string(config_path)?;
            toml::from_str(&content)?
        } else {
            Config::default()
        };

        // 2. 命令行参数覆盖
        if let Some(node_id) = args.node_id {
            config.node.node_id = node_id;
        }
        if let Some(listen_addr) = args.listen_addr {
            config.node.listen_addr = listen_addr;
        }
        if let Some(data_dir) = args.data_dir {
            config.node.data_dir = data_dir;
        }
        if let Some(http_port) = args.http_port {
            config.http.port = http_port;
        }
        if let Some(log_level) = args.log_level {
            config.logging.level = log_level;
        }

        // 3. 验证配置
        config.validate()?;

        Ok(config)
    }

    /// 验证配置合法性
    fn validate(&self) -> anyhow::Result<()> {
        if self.node.node_id == 0 {
            anyhow::bail!("node_id must be greater than 0");
        }

        if self.node.listen_addr.is_empty() {
            anyhow::bail!("listen_addr cannot be empty");
        }

        if self.raft.heartbeat_interval_ms >= self.raft.election_timeout_ms {
            anyhow::bail!("heartbeat_interval_ms must be less than election_timeout_ms");
        }

        if self.http.enabled && self.http.port == 0 {
            anyhow::bail!("HTTP port cannot be 0 when HTTP is enabled");
        }

        let mut seen_node_ids = HashSet::new();
        let mut seen_addrs = HashSet::new();
        for p in &self.cluster.peers {
            if p.node_id == 0 {
                anyhow::bail!("cluster.peers.node_id must be greater than 0");
            }
            if p.addr.trim().is_empty() {
                anyhow::bail!("cluster.peers.addr cannot be empty");
            }
            if p.node_id == self.node.node_id {
                anyhow::bail!("cluster.peers must not include current node_id");
            }
            if p.addr == self.node.listen_addr {
                anyhow::bail!("cluster.peers must not reuse current node listen_addr");
            }
            if !seen_node_ids.insert(p.node_id) {
                anyhow::bail!("cluster.peers contains duplicate node_id: {}", p.node_id);
            }
            if !seen_addrs.insert(p.addr.clone()) {
                anyhow::bail!("cluster.peers contains duplicate addr: {}", p.addr);
            }
        }

        if let Some(expected) = self.cluster.expected_voters {
            if expected == 0 {
                anyhow::bail!("cluster.expected_voters must be greater than 0");
            }

            let actual = 1 + self.cluster.peers.len();
            if expected != actual {
                anyhow::bail!(
                    "cluster.expected_voters mismatch: expected={}, actual={} (self + peers)",
                    expected,
                    actual
                );
            }
        }

        Ok(())
    }

    /// 构建 OpenRaft 初始化 membership（包含本节点 + 配置 peers）。
    pub fn cluster_members(&self) -> BTreeMap<u64, openraft::BasicNode> {
        let mut out = BTreeMap::new();
        out.insert(
            self.node.node_id,
            openraft::BasicNode::new(&self.node.listen_addr),
        );
        for p in &self.cluster.peers {
            out.insert(p.node_id, openraft::BasicNode::new(&p.addr));
        }
        out
    }

    /// 构建 network resolver 地址映射（包含本节点 + peers）。
    pub fn resolver_addresses(&self) -> HashMap<u64, String> {
        let mut out = HashMap::new();
        out.insert(self.node.node_id, self.node.listen_addr.clone());
        for p in &self.cluster.peers {
            out.insert(p.node_id, p.addr.clone());
        }
        out
    }

    /// 创建数据目录（如果不存在）
    pub fn ensure_data_dir(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.node.data_dir)?;
        Ok(())
    }

    /// 启动前自检：目录可写、端口可绑定。
    pub fn preflight_check(&self) -> anyhow::Result<()> {
        self.ensure_data_dir()?;

        // 目录可写性检查。
        let probe = self.node.data_dir.join(".write_probe");
        std::fs::write(&probe, b"ok")
            .map_err(|e| anyhow::anyhow!("data_dir is not writable: {}", e))?;
        let _ = std::fs::remove_file(&probe);

        // gRPC 监听地址端口可用性检查。
        Self::check_bindable(&self.node.listen_addr)?;

        // HTTP 端口可用性检查。
        if self.http.enabled {
            let http_addr = format!("0.0.0.0:{}", self.http.port);
            Self::check_bindable(&http_addr)?;
        }

        Ok(())
    }

    fn check_bindable(addr: &str) -> anyhow::Result<()> {
        let listener = std::net::TcpListener::bind(addr)
            .map_err(|e| anyhow::anyhow!("address not bindable ({}): {}", addr, e))?;
        drop(listener);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.node.node_id, 1);
        assert_eq!(config.http.port, 8080);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_node_id_zero() {
        let mut config = Config::default();
        config.node.node_id = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_heartbeat_interval() {
        let mut config = Config::default();
        config.raft.heartbeat_interval_ms = 5000;
        config.raft.election_timeout_ms = 3000;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_preflight_disabled_http() {
        let mut config = Config::default();
        config.http.enabled = false;
        assert!(config.preflight_check().is_ok());
    }

    #[test]
    fn test_cluster_members_contains_self() {
        let config = Config::default();
        let members = config.cluster_members();
        assert!(members.contains_key(&config.node.node_id));
    }

    #[test]
    fn test_validate_cluster_duplicate_peer_id() {
        let mut config = Config::default();
        config.cluster.peers = vec![
            PeerConfig {
                node_id: 2,
                addr: "127.0.0.1:50052".to_string(),
            },
            PeerConfig {
                node_id: 2,
                addr: "127.0.0.1:50053".to_string(),
            },
        ];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_cluster_duplicate_peer_addr() {
        let mut config = Config::default();
        config.cluster.peers = vec![
            PeerConfig {
                node_id: 2,
                addr: "127.0.0.1:50052".to_string(),
            },
            PeerConfig {
                node_id: 3,
                addr: "127.0.0.1:50052".to_string(),
            },
        ];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_cluster_peer_reuse_self_addr() {
        let mut config = Config::default();
        config.cluster.peers = vec![PeerConfig {
            node_id: 2,
            addr: config.node.listen_addr.clone(),
        }];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cluster_members_expected_count() {
        let mut config = Config::default();
        config.cluster.bootstrap = true;
        config.cluster.peers = vec![
            PeerConfig {
                node_id: 2,
                addr: "127.0.0.1:50052".to_string(),
            },
            PeerConfig {
                node_id: 3,
                addr: "127.0.0.1:50053".to_string(),
            },
        ];

        let members = config.cluster_members();
        assert_eq!(members.len(), 3);
        assert!(members.contains_key(&1));
        assert!(members.contains_key(&2));
        assert!(members.contains_key(&3));
    }

    #[test]
    fn test_resolver_addresses_mapping() {
        let mut config = Config::default();
        config.cluster.peers = vec![
            PeerConfig {
                node_id: 2,
                addr: "127.0.0.1:50052".to_string(),
            },
            PeerConfig {
                node_id: 3,
                addr: "127.0.0.1:50053".to_string(),
            },
        ];

        let addrs = config.resolver_addresses();
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs.get(&1).map(String::as_str), Some("127.0.0.1:50051"));
        assert_eq!(addrs.get(&2).map(String::as_str), Some("127.0.0.1:50052"));
        assert_eq!(addrs.get(&3).map(String::as_str), Some("127.0.0.1:50053"));
    }

    #[test]
    fn test_validate_expected_voters_match() {
        let mut config = Config::default();
        config.cluster.peers = vec![
            PeerConfig {
                node_id: 2,
                addr: "127.0.0.1:50052".to_string(),
            },
            PeerConfig {
                node_id: 3,
                addr: "127.0.0.1:50053".to_string(),
            },
        ];
        config.cluster.expected_voters = Some(3);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_expected_voters_mismatch() {
        let mut config = Config::default();
        config.cluster.peers = vec![PeerConfig {
            node_id: 2,
            addr: "127.0.0.1:50052".to_string(),
        }];
        config.cluster.expected_voters = Some(3);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_expected_voters_zero() {
        let mut config = Config::default();
        config.cluster.expected_voters = Some(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_expected_voters_mismatch_even_without_bootstrap() {
        let mut config = Config::default();
        config.cluster.bootstrap = false;
        config.cluster.peers = vec![PeerConfig {
            node_id: 2,
            addr: "127.0.0.1:50052".to_string(),
        }];
        config.cluster.expected_voters = Some(3);
        assert!(config.validate().is_err());
    }
}
