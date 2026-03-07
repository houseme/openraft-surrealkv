//! Raft Node management and initialization.

use crate::config::Config;
use crate::network::GrpcRaftNetworkFactory;
use crate::storage::SurrealStorage;
use crate::types::{KVRequest, KVResponse, RaftTypeConfig};
use openraft::rt::WatchReceiver;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Raft node wrapper (Phase 5.2)
pub struct RaftNode {
    pub is_standalone: bool,
    pub node_id: u64,
    storage: Arc<SurrealStorage>,
    raft: Option<openraft::Raft<RaftTypeConfig>>,
}

impl RaftNode {
    /// Create a new RaftNode and initialize the OpenRaft runtime.
    ///
    /// - `config`: application configuration used to build Raft config
    /// - `storage`: storage adapter implementing OpenRaft traits
    /// - `network_factory`: gRPC network factory used by Raft for RPCs
    pub async fn new(
        config: &Config,
        storage: Arc<SurrealStorage>,
        network_factory: Arc<GrpcRaftNetworkFactory>,
    ) -> anyhow::Result<Self> {
        let mut raft_cfg = openraft::Config {
            cluster_name: "openraft-surrealkv".to_string(),
            heartbeat_interval: config.raft.heartbeat_interval_ms,
            election_timeout_min: config.raft.election_timeout_ms,
            election_timeout_max: config
                .raft
                .election_timeout_ms
                .saturating_add(config.raft.heartbeat_interval_ms.max(1)),
            max_payload_entries: config.raft.max_payload_entries,
            ..Default::default()
        };
        raft_cfg = raft_cfg
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid openraft config: {}", e))?;

        let raft = openraft::Raft::new(
            config.node.node_id,
            Arc::new(raft_cfg),
            network_factory.as_ref().clone(),
            storage.as_ref().clone(),
            storage.as_ref().clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("create raft node failed: {}", e))?;

        Ok(RaftNode {
            is_standalone: false,
            node_id: config.node.node_id,
            storage,
            raft: Some(raft),
        })
    }

    /// Initialize the Raft cluster with the given members.
    ///
    /// - `members`: a map of node IDs to `BasicNode` instances representing the cluster members.
    ///   The current node must be included in the members with its correct ID.
    ///
    /// Returns an error if the members list is empty or if the current node is not included.
    pub async fn initialize_cluster(
        &self,
        members: BTreeMap<u64, openraft::BasicNode>,
    ) -> anyhow::Result<()> {
        if members.is_empty() {
            anyhow::bail!("raft initialize members cannot be empty");
        }
        if !members.contains_key(&self.node_id) {
            anyhow::bail!("raft initialize members must include current node");
        }

        if let Some(raft) = &self.raft {
            raft.initialize(members)
                .await
                .map_err(|e| anyhow::anyhow!("raft initialize failed: {}", e))?;
        }
        Ok(())
    }

    /// Create a RaftNode in standalone mode (no Raft consensus).
    /// Useful for local development or single-node deployments.
    pub async fn new_standalone(
        config: &Config,
        storage: Arc<SurrealStorage>,
    ) -> anyhow::Result<Self> {
        tracing::warn!(
            node_id = config.node.node_id,
            "Running in standalone mode (no Raft consensus)"
        );
        Ok(RaftNode {
            is_standalone: true,
            node_id: config.node.node_id,
            storage,
            raft: None,
        })
    }

    /// Unified client write path.
    ///
    /// Tries to perform an OpenRaft `client_write` when Raft is enabled; if the call
    /// fails with common leader/forwarding errors, falls back to applying the request
    /// directly to the local state machine (useful for standalone mode or transient leader issues).
    pub async fn client_write(&self, req: KVRequest) -> anyhow::Result<KVResponse> {
        if let Some(raft) = &self.raft {
            match raft.client_write(req.clone()).await {
                Ok(resp) => return Ok(resp.data),
                Err(e) => {
                    let msg = e.to_string();
                    let fallback_allowed = {
                        let lower = msg.to_lowercase();
                        lower.contains("forward")
                            || lower.contains("leader")
                            || lower.contains("temporarily unavailable")
                            || lower.contains("unavailable")
                    };

                    if fallback_allowed {
                        tracing::warn!(
                            node_id = self.node_id,
                            error = %msg,
                            "raft client_write fallback triggered (strategy C)"
                        );
                    } else {
                        return Err(anyhow::anyhow!("raft client_write failed: {}", msg));
                    }
                }
            }
        }

        self.storage
            .apply_request(&req)
            .await
            .map_err(|e| anyhow::anyhow!("client_write fallback failed: {}", e))
    }

    pub fn raft_handle(&self) -> Option<openraft::Raft<RaftTypeConfig>> {
        self.raft.clone()
    }

    pub fn is_real_raft(&self) -> bool {
        self.raft.is_some() && !self.is_standalone
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        if let Some(raft) = &self.raft {
            raft.shutdown()
                .await
                .map_err(|e| anyhow::anyhow!("raft shutdown failed: {}", e))?;
        }
        Ok(())
    }

    /// 获取当前任期
    pub async fn current_term(&self) -> u64 {
        if let Some(raft) = &self.raft {
            let metrics = raft.metrics();
            return metrics.borrow_watched().current_term;
        }
        0
    }

    /// 获取当前角色（Leader/Follower/Candidate）
    pub async fn current_role(&self) -> String {
        if let Some(raft) = &self.raft {
            let metrics = raft.metrics();
            return format!("{:?}", metrics.borrow_watched().state);
        }

        if self.is_standalone {
            "Standalone".to_string()
        } else {
            "Running".to_string()
        }
    }

    /// 获取已应用索引
    pub async fn applied_index(&self) -> u64 {
        self.storage
            .metadata()
            .get_applied_state()
            .await
            .last_applied_index
    }

    /// 读取状态机中的 key（主要用于集成测试与诊断）
    pub async fn read_key(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        self.storage
            .read(key)
            .await
            .map_err(|e| anyhow::anyhow!("read key failed: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raft_config_creation() {
        let config = Config::default();
        let mut raft_config = openraft::Config::default();
        raft_config.heartbeat_interval = config.raft.heartbeat_interval_ms;
        assert_eq!(raft_config.heartbeat_interval, 500);
    }

    #[tokio::test]
    async fn test_standalone_raft_node() -> anyhow::Result<()> {
        let config = Config::default();
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_standalone".into())
                .build()?,
        );
        let storage = Arc::new(SurrealStorage::new(tree).await?);
        let node = RaftNode::new_standalone(&config, storage).await?;

        assert!(node.is_standalone);
        assert_eq!(node.current_role().await, "Standalone");
        assert_eq!(node.current_term().await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn test_client_write_path() -> anyhow::Result<()> {
        let config = Config::default();
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_client_write_path".into())
                .build()?,
        );
        let storage = Arc::new(SurrealStorage::new(tree).await?);
        let node = RaftNode::new_standalone(&config, storage.clone()).await?;

        let _ = node
            .client_write(KVRequest::Set {
                key: "k".to_string(),
                value: b"v".to_vec(),
            })
            .await?;

        assert_eq!(storage.read("k").await?, Some(b"v".to_vec()));
        Ok(())
    }

    #[tokio::test]
    async fn test_initialize_cluster_rejects_empty_members() -> anyhow::Result<()> {
        let config = Config::default();
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_init_cluster_empty".into())
                .build()?,
        );
        let storage = Arc::new(SurrealStorage::new(tree).await?);
        let node = RaftNode::new_standalone(&config, storage).await?;

        let err = node.initialize_cluster(BTreeMap::new()).await.unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
        Ok(())
    }

    #[tokio::test]
    async fn test_initialize_cluster_rejects_missing_self() -> anyhow::Result<()> {
        let config = Config::default();
        let tree = Arc::new(
            surrealkv::TreeBuilder::new()
                .with_path("target/test_init_cluster_missing_self".into())
                .build()?,
        );
        let storage = Arc::new(SurrealStorage::new(tree).await?);
        let node = RaftNode::new_standalone(&config, storage).await?;

        let mut members = BTreeMap::new();
        members.insert(2, openraft::BasicNode::new("127.0.0.1:50052"));

        let err = node.initialize_cluster(members).await.unwrap_err();
        assert!(err.to_string().contains("must include current node"));
        Ok(())
    }
}
