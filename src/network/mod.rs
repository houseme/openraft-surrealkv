//! Tonic gRPC network layer and Raft RPC implementation.
//!
//! This module provides:
//! - RaftNetworkFactory for managing node connections
//! - RaftNetwork implementation for single node communication
//! - gRPC service handlers for AppendEntries, RequestVote, InstallSnapshot
//! - Connection pooling, timeouts, retries
//!
//! Phase 1 implementation (Tonic v2).
//! Phase 1.5+ implementation (Streaming snapshot support).

use crate::types::{NodeId, RaftTypeConfig};
use openraft::errors::{NetworkError, RPCError, ReplicationClosed, StreamingError};
use openraft::network::{RPCOption, RaftNetworkFactory, RaftNetworkV2};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, SnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::{Snapshot, Vote};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

pub mod client;
pub mod server;

/// gRPC-based RaftNetworkFactory implementation.
#[derive(Clone)]
pub struct GrpcRaftNetworkFactory {
    connection_pool: client::ConnectionPool,
}

impl GrpcRaftNetworkFactory {
    /// Create a new factory with address resolver and default client config.
    pub fn new(resolver: Arc<dyn client::AddressResolver + Send + Sync>) -> Self {
        Self {
            connection_pool: client::ConnectionPool::new(resolver),
        }
    }

    /// Create a new factory with custom gRPC client settings.
    pub fn with_config(
        resolver: Arc<dyn client::AddressResolver + Send + Sync>,
        config: client::ClientConfig,
    ) -> Self {
        Self {
            connection_pool: client::ConnectionPool::with_config(resolver, config),
        }
    }

    /// Create a new factory with static address map.
    pub fn with_static_addresses(addresses: HashMap<NodeId, String>) -> Self {
        let resolver = Arc::new(client::StaticAddressResolver::new(addresses));
        Self::new(resolver)
    }

    /// Expose cached connection count for integration tests/metrics wiring.
    pub async fn cached_connection_count(&self) -> usize {
        self.connection_pool.client_count().await
    }
}

impl RaftNetworkFactory<RaftTypeConfig> for GrpcRaftNetworkFactory {
    type Network = GrpcRaftNetwork;

    async fn new_client(&mut self, target: NodeId, _node: &openraft::BasicNode) -> Self::Network {
        GrpcRaftNetwork {
            target,
            connection_pool: self.connection_pool.clone(),
        }
    }
}

/// gRPC-based RaftNetwork implementation.
#[derive(Clone)]
pub struct GrpcRaftNetwork {
    target: NodeId,
    connection_pool: client::ConnectionPool,
}

impl GrpcRaftNetwork {
    fn to_rpc_err(err: &crate::error::Error) -> RPCError<RaftTypeConfig> {
        RPCError::Network(NetworkError::new(err))
    }
}

impl RaftNetworkV2<RaftTypeConfig> for GrpcRaftNetwork {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<RaftTypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<RaftTypeConfig>, RPCError<RaftTypeConfig>> {
        let mut client = self
            .connection_pool
            .get_client(self.target)
            .await
            .map_err(|e| Self::to_rpc_err(&e))?;

        client
            .append_entries(req)
            .await
            .map_err(|e| Self::to_rpc_err(&e))
    }

    async fn vote(
        &mut self,
        req: VoteRequest<RaftTypeConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<RaftTypeConfig>, RPCError<RaftTypeConfig>> {
        let mut client = self
            .connection_pool
            .get_client(self.target)
            .await
            .map_err(|e| Self::to_rpc_err(&e))?;

        client
            .request_vote(req)
            .await
            .map_err(|e| Self::to_rpc_err(&e))
    }

    async fn full_snapshot(
        &mut self,
        vote: Vote<RaftTypeConfig>,
        snapshot: Snapshot<RaftTypeConfig>,
        cancel: impl Future<Output = ReplicationClosed> + Send + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<RaftTypeConfig>, StreamingError<RaftTypeConfig>> {
        let mut cancel = std::pin::pin!(cancel);

        let mut client = self
            .connection_pool
            .get_client(self.target)
            .await
            .map_err(|e| StreamingError::from(Self::to_rpc_err(&e)))?;

        let chunk_size = option.snapshot_chunk_size().unwrap_or(1024 * 1024).max(1);
        let bytes = snapshot.snapshot.into_inner();
        let mut offset = 0usize;
        let mut last_vote = vote.clone();

        if bytes.is_empty() {
            let req = InstallSnapshotRequest {
                vote,
                meta: snapshot.meta,
                offset: 0,
                data: vec![],
                done: true,
            };

            let resp = tokio::select! {
                closed = &mut cancel => return Err(StreamingError::Closed(closed)),
                resp = client.install_snapshot(req) => resp,
            }
            .map_err(|e| StreamingError::from(Self::to_rpc_err(&e)))?;

            return Ok(SnapshotResponse::new(resp.vote));
        }

        while offset < bytes.len() {
            let end = std::cmp::min(offset + chunk_size, bytes.len());
            let req = InstallSnapshotRequest {
                vote: vote.clone(),
                meta: snapshot.meta.clone(),
                offset: offset as u64,
                data: bytes[offset..end].to_vec(),
                done: end == bytes.len(),
            };

            let resp = tokio::select! {
                closed = &mut cancel => return Err(StreamingError::Closed(closed)),
                resp = client.install_snapshot(req) => resp,
            }
            .map_err(|e| StreamingError::from(Self::to_rpc_err(&e)))?;

            last_vote = resp.vote;
            offset = end;
        }

        Ok(SnapshotResponse::new(last_vote))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_factory_creation() {
        let resolver = Arc::new(client::StaticAddressResolver::new(HashMap::new()));
        let _factory = GrpcRaftNetworkFactory::new(resolver);
    }
}
