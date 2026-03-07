//! gRPC client implementation for Raft network communication.
//!
//! This module provides:
//! - RaftServiceClient wrapper with connection management
//! - Serialization/deserialization using postcard
//! - Timeout and retry logic

use crate::error::{Error, Result};
use crate::types::{NodeId, RaftTypeConfig};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tonic::transport::Channel;
use tracing::{debug, info, warn};

use crate::proto::{RaftMessage, raft_service_client::RaftServiceClient};

/// Runtime gRPC client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_retries: usize,
    pub max_message_size: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(2),
            max_retries: 2,
            max_message_size: 64 * 1024 * 1024,
        }
    }
}

/// gRPC client wrapper with connection caching
#[derive(Clone)]
pub struct RaftGrpcClient {
    /// Target node ID
    target: NodeId,
    /// gRPC client instance
    client: RaftServiceClient<Channel>,
    /// Request timeout and retry policy.
    config: ClientConfig,
}

impl RaftGrpcClient {
    /// Create a new client connected to the target node
    pub async fn connect(target: NodeId, addr: &str, config: ClientConfig) -> Result<Self> {
        let addr_with_scheme = if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            format!("http://{}", addr)
        };

        let endpoint = tonic::transport::Endpoint::from_shared(addr_with_scheme.clone())
            .map_err(|e| Error::Network(format!("Invalid address {}: {}", addr, e)))?
            .connect_timeout(config.connect_timeout);

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| Error::Network(format!("Failed to connect to {}: {}", addr, e)))?;

        let client = RaftServiceClient::new(channel)
            .max_decoding_message_size(config.max_message_size)
            .max_encoding_message_size(config.max_message_size);

        info!(
            target,
            addr,
            request_timeout_ms = config.request_timeout.as_millis(),
            max_retries = config.max_retries,
            "Connected to raft peer"
        );

        Ok(Self {
            target,
            client,
            config,
        })
    }

    /// Send AppendEntries RPC
    pub async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<RaftTypeConfig>,
    ) -> Result<AppendEntriesResponse<RaftTypeConfig>> {
        let payload = postcard::to_stdvec(&req)
            .map_err(|e| Error::Serialization(format!("Failed to serialize request: {}", e)))?;

        let mut attempt = 0usize;
        let response = loop {
            let request = tonic::Request::new(RaftMessage {
                payload: payload.clone(),
            });

            let call = tokio::time::timeout(
                self.config.request_timeout,
                self.client.append_entries(request),
            )
            .await;

            match call {
                Ok(Ok(resp)) => break resp,
                Ok(Err(e)) => {
                    if attempt >= self.config.max_retries {
                        return Err(Error::Network(format!(
                            "AppendEntries RPC to node {} failed after {} attempts: {}",
                            self.target,
                            attempt + 1,
                            e
                        )));
                    }
                    attempt += 1;
                    warn!(target = self.target, attempt, error = %e, "retrying append_entries rpc");
                }
                Err(_) => {
                    if attempt >= self.config.max_retries {
                        return Err(Error::Network(format!(
                            "AppendEntries RPC to node {} timed out after {} attempts",
                            self.target,
                            attempt + 1
                        )));
                    }
                    attempt += 1;
                    warn!(
                        target = self.target,
                        attempt, "append_entries rpc timed out, retrying"
                    );
                }
            }
        };

        let resp: AppendEntriesResponse<RaftTypeConfig> =
            postcard::from_bytes(&response.into_inner().payload).map_err(|e| {
                Error::Serialization(format!("Failed to deserialize response: {}", e))
            })?;

        debug!(target = self.target, "AppendEntries success");
        Ok(resp)
    }

    /// Send RequestVote RPC
    pub async fn request_vote(
        &mut self,
        req: VoteRequest<RaftTypeConfig>,
    ) -> Result<VoteResponse<RaftTypeConfig>> {
        let payload = postcard::to_stdvec(&req)
            .map_err(|e| Error::Serialization(format!("Failed to serialize request: {}", e)))?;

        let mut attempt = 0usize;
        let response = loop {
            let request = tonic::Request::new(RaftMessage {
                payload: payload.clone(),
            });

            let call = tokio::time::timeout(
                self.config.request_timeout,
                self.client.request_vote(request),
            )
            .await;

            match call {
                Ok(Ok(resp)) => break resp,
                Ok(Err(e)) => {
                    if attempt >= self.config.max_retries {
                        return Err(Error::Network(format!(
                            "RequestVote RPC to node {} failed after {} attempts: {}",
                            self.target,
                            attempt + 1,
                            e
                        )));
                    }
                    attempt += 1;
                    warn!(target = self.target, attempt, error = %e, "retrying request_vote rpc");
                }
                Err(_) => {
                    if attempt >= self.config.max_retries {
                        return Err(Error::Network(format!(
                            "RequestVote RPC to node {} timed out after {} attempts",
                            self.target,
                            attempt + 1
                        )));
                    }
                    attempt += 1;
                    warn!(
                        target = self.target,
                        attempt, "request_vote rpc timed out, retrying"
                    );
                }
            }
        };

        let resp: VoteResponse<RaftTypeConfig> =
            postcard::from_bytes(&response.into_inner().payload).map_err(|e| {
                Error::Serialization(format!("Failed to deserialize response: {}", e))
            })?;

        debug!(target = self.target, "RequestVote success");
        Ok(resp)
    }

    /// Send InstallSnapshot RPC
    pub async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<RaftTypeConfig>,
    ) -> Result<InstallSnapshotResponse<RaftTypeConfig>> {
        let payload = postcard::to_stdvec(&req)
            .map_err(|e| Error::Serialization(format!("Failed to serialize request: {}", e)))?;

        let mut attempt = 0usize;
        let response = loop {
            let request = tonic::Request::new(RaftMessage {
                payload: payload.clone(),
            });

            let call = tokio::time::timeout(
                self.config.request_timeout,
                self.client.install_snapshot(request),
            )
            .await;

            match call {
                Ok(Ok(resp)) => break resp,
                Ok(Err(e)) => {
                    if attempt >= self.config.max_retries {
                        return Err(Error::Network(format!(
                            "InstallSnapshot RPC to node {} failed after {} attempts: {}",
                            self.target,
                            attempt + 1,
                            e
                        )));
                    }
                    attempt += 1;
                    warn!(target = self.target, attempt, error = %e, "retrying install_snapshot rpc");
                }
                Err(_) => {
                    if attempt >= self.config.max_retries {
                        return Err(Error::Network(format!(
                            "InstallSnapshot RPC to node {} timed out after {} attempts",
                            self.target,
                            attempt + 1
                        )));
                    }
                    attempt += 1;
                    warn!(
                        target = self.target,
                        attempt, "install_snapshot rpc timed out, retrying"
                    );
                }
            }
        };

        let resp: InstallSnapshotResponse<RaftTypeConfig> =
            postcard::from_bytes(&response.into_inner().payload).map_err(|e| {
                Error::Serialization(format!("Failed to deserialize response: {}", e))
            })?;

        debug!(target = self.target, "InstallSnapshot success");
        Ok(resp)
    }
}

/// Connection pool for managing client connections
#[derive(Clone)]
pub struct ConnectionPool {
    /// Map of node ID to client
    clients: Arc<Mutex<HashMap<NodeId, RaftGrpcClient>>>,
    /// Node address resolver (node_id -> address)
    address_resolver: Arc<dyn AddressResolver + Send + Sync>,
    config: ClientConfig,
}

/// Trait for resolving node addresses
#[async_trait::async_trait]
pub trait AddressResolver {
    async fn resolve(&self, node_id: NodeId) -> Result<String>;
}

/// Simple address resolver using a static map
pub struct StaticAddressResolver {
    addresses: HashMap<NodeId, String>,
}

impl StaticAddressResolver {
    pub fn new(addresses: HashMap<NodeId, String>) -> Self {
        Self { addresses }
    }
}

#[async_trait::async_trait]
impl AddressResolver for StaticAddressResolver {
    async fn resolve(&self, node_id: NodeId) -> Result<String> {
        self.addresses
            .get(&node_id)
            .cloned()
            .ok_or_else(|| Error::Network(format!("No address found for node {}", node_id)))
    }
}

impl ConnectionPool {
    pub fn new(resolver: Arc<dyn AddressResolver + Send + Sync>) -> Self {
        Self::with_config(resolver, ClientConfig::default())
    }

    pub fn with_config(
        resolver: Arc<dyn AddressResolver + Send + Sync>,
        config: ClientConfig,
    ) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            address_resolver: resolver,
            config,
        }
    }

    /// Get or create a client for the target node.
    pub async fn get_client(&self, target: NodeId) -> Result<RaftGrpcClient> {
        let mut clients = self.clients.lock().await;
        if let Some(existing) = clients.get(&target).cloned() {
            return Ok(existing);
        }

        let addr = self.address_resolver.resolve(target).await?;
        let created = RaftGrpcClient::connect(target, &addr, self.config.clone()).await?;

        clients.insert(target, created.clone());
        Ok(created)
    }

    /// Number of currently cached node connections.
    pub async fn client_count(&self) -> usize {
        self.clients.lock().await.len()
    }

    /// Remove a client (e.g., on connection failure)
    pub async fn remove_client(&self, target: NodeId) {
        let mut clients = self.clients.lock().await;
        clients.remove(&target);
        warn!("Removed client connection for node {}", target);
    }
}
