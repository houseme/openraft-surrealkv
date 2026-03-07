use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

/// Configuration for the OpenRaft-SurrealKV distributed KV service.
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

/// Node configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Node ID (1, 2, 3, ...)
    pub node_id: u64,
    /// gRPC listen address.
    pub listen_addr: String,
    /// Data directory.
    pub data_dir: PathBuf,
}

/// HTTP API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// Whether to enable the HTTP API.
    pub enabled: bool,
    /// HTTP listen port.
    pub port: u16,
}

/// Raft configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaftConfig {
    /// Heartbeat interval (milliseconds).
    pub heartbeat_interval_ms: u64,
    /// Election timeout (milliseconds).
    pub election_timeout_ms: u64,
    /// Maximum batched log entries per payload.
    pub max_payload_entries: u64,
}

/// Snapshot configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotConfig {
    /// Checkpoint interval (seconds).
    pub checkpoint_interval_secs: u64,
    /// Maximum delta chain length.
    pub max_delta_chain: usize,
    /// Maximum cumulative delta bytes (MB).
    pub max_delta_bytes_mb: u64,
}

/// Storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Whether to enable compression.
    pub enable_compression: bool,
    /// Flush interval (milliseconds).
    pub flush_interval_ms: u64,
}

/// Logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (`trace`/`debug`/`info`/`warn`/`error`).
    pub level: String,
    /// Log output format (`json`/`text`).
    pub format: String,
}

/// Metrics configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Whether to enable metrics export.
    pub enabled: bool,
    /// Metrics listen address.
    pub listen_addr: String,
}

/// Cluster configuration (multi-node bootstrap/join).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClusterConfig {
    /// Whether the current node performs bootstrap initialization.
    pub bootstrap: bool,
    /// Expected voter count (self + peers), used to prevent misconfiguration.
    pub expected_voters: Option<usize>,
    /// Known cluster peers (excluding the current node).
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

/// Peer node configuration.
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

/// Command-line arguments.
#[derive(Debug, Clone, Parser)]
#[command(name = "openraft-surrealkv")]
#[command(about = "OpenRaft + SurrealKV distributed KV service", long_about = None)]
pub struct CliArgs {
    /// Configuration file path.
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Node ID (overrides config file).
    #[arg(long, env = "NODE_ID")]
    pub node_id: Option<u64>,

    /// Listen address (overrides config file).
    #[arg(long, env = "LISTEN_ADDR")]
    pub listen_addr: Option<String>,

    /// Data directory (overrides config file).
    #[arg(long, env = "DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// HTTP port (overrides config file).
    #[arg(long, env = "HTTP_PORT")]
    pub http_port: Option<u16>,

    /// Log level (overrides config file).
    #[arg(long, env = "LOG_LEVEL")]
    pub log_level: Option<String>,
}

impl Config {
    /// Load configuration from CLI arguments and config file.
    ///
    /// Precedence: CLI arguments > environment variables > config file > defaults.
    pub fn load() -> anyhow::Result<Self> {
        let args = CliArgs::parse();

        // 1. Load config file or fall back to defaults.
        let mut config = if let Some(config_path) = &args.config {
            let content = std::fs::read_to_string(config_path)?;
            toml::from_str(&content)?
        } else {
            Config::default()
        };

        // 2. Apply CLI overrides.
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

        // 3. Validate final config.
        config.validate()?;

        Ok(config)
    }

    /// Validate configuration values.
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

    /// Build OpenRaft initial membership (self + configured peers).
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

    /// Build the address map for the network resolver (self + peers).
    pub fn resolver_addresses(&self) -> HashMap<u64, String> {
        let mut out = HashMap::new();
        out.insert(self.node.node_id, self.node.listen_addr.clone());
        for p in &self.cluster.peers {
            out.insert(p.node_id, p.addr.clone());
        }
        out
    }

    /// Create the data directory if it does not exist.
    pub fn ensure_data_dir(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.node.data_dir)?;
        Ok(())
    }

    /// Startup preflight checks: writable directory and bindable ports.
    pub fn preflight_check(&self) -> anyhow::Result<()> {
        self.ensure_data_dir()?;

        // Check data directory writability.
        let probe = self.node.data_dir.join(".write_probe");
        std::fs::write(&probe, b"ok")
            .map_err(|e| anyhow::anyhow!("data_dir is not writable: {}", e))?;
        let _ = std::fs::remove_file(&probe);

        // Check gRPC listen address availability.
        Self::check_bindable(&self.node.listen_addr)?;

        // Check HTTP port availability.
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
