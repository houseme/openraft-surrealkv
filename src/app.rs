//! Raft Node 管理与初始化

use crate::config::Config;
use crate::network::GrpcRaftNetworkFactory;
use crate::storage::SurrealStorage;
use crate::types::{KVRequest, KVResponse, RaftTypeConfig};
use std::sync::Arc;

/// Raft 节点包装器（Phase 5.2）
pub struct RaftNode {
    pub is_standalone: bool,
    pub node_id: u64,
    storage: Arc<SurrealStorage>,
    raft: Option<openraft::Raft<RaftTypeConfig>>,
}

impl RaftNode {
    /// 创建新的 Raft Node
    /// NOTE: Currently creates standalone mode due to OpenRaft trait implementation in progress
    pub async fn new(
        config: &Config,
        storage: Arc<SurrealStorage>,
        _network_factory: Arc<GrpcRaftNetworkFactory>,
    ) -> anyhow::Result<Self> {
        tracing::warn!(
            node_id = config.node.node_id,
            "Raft trait implementation in progress, running in standalone mode"
        );
        Self::new_standalone(config, storage).await
    }

    /// 创建单机模式（无 Raft，仅存储）
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

    /// 统一客户端写路径（优先真实 OpenRaft client_write，失败时 fallback）。
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

    /// 获取当前任期
    pub async fn current_term(&self) -> u64 {
        0
    }

    /// 获取当前角色（Leader/Follower/Candidate）
    pub async fn current_role(&self) -> String {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raft_config_creation() {
        let config = Config::default();
        let mut raft_config = RaftConfig::default();
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
}
