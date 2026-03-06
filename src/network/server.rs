//! gRPC server implementation for Raft network communication.
//!
//! This module provides:
//! - RaftService server implementation
//! - Serialization/deserialization using postcard
//! - Integration with OpenRaft RaftNetwork

use crate::error::{Error, Result};
use crate::types::RaftTypeConfig;
use openraft::errors::RaftError;
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use std::future::{Future, pending};
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::{debug, error};

use crate::proto::raft::raft_service_server::RaftServiceServer;
use crate::proto::raft::{RaftMessage, raft_service_server::RaftService};

/// Trait for handling Raft RPCs.
#[async_trait::async_trait]
pub trait RaftServiceHandler: Send + Sync + 'static {
    async fn append_entries(
        &self,
        req: AppendEntriesRequest<RaftTypeConfig>,
    ) -> std::result::Result<AppendEntriesResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>>;

    async fn vote(
        &self,
        req: VoteRequest<RaftTypeConfig>,
    ) -> std::result::Result<VoteResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>>;

    async fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<RaftTypeConfig>,
    ) -> std::result::Result<InstallSnapshotResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>>;
}

#[async_trait::async_trait]
impl RaftServiceHandler for openraft::Raft<RaftTypeConfig> {
    async fn append_entries(
        &self,
        req: AppendEntriesRequest<RaftTypeConfig>,
    ) -> std::result::Result<AppendEntriesResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>> {
        self.append_entries(req).await
    }

    async fn vote(
        &self,
        req: VoteRequest<RaftTypeConfig>,
    ) -> std::result::Result<VoteResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>> {
        self.vote(req).await
    }

    async fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<RaftTypeConfig>,
    ) -> std::result::Result<InstallSnapshotResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>>
    {
        self.install_snapshot(req).await
    }
}

/// gRPC server implementation for RaftService
pub struct RaftGrpcServer<H> {
    /// The Raft service handler
    handler: Arc<H>,
}

impl<H> RaftGrpcServer<H>
where
    H: RaftServiceHandler,
{
    pub fn new(handler: Arc<H>) -> Self {
        Self { handler }
    }
}

#[tonic::async_trait]
impl<H> RaftService for RaftGrpcServer<H>
where
    H: RaftServiceHandler,
{
    async fn append_entries(
        &self,
        request: Request<RaftMessage>,
    ) -> std::result::Result<Response<RaftMessage>, Status> {
        let req_payload = request.into_inner().payload;

        let req: AppendEntriesRequest<RaftTypeConfig> = postcard::from_bytes(&req_payload)
            .map_err(|e| {
                error!(error = %e, "Failed to deserialize AppendEntriesRequest");
                Status::invalid_argument(format!("Deserialization failed: {}", e))
            })?;

        debug!(vote = %req.vote, "Received AppendEntries");

        let resp = self.handler.append_entries(req).await.map_err(|e| {
            error!(error = ?e, "AppendEntries failed");
            Status::internal(format!("Raft operation failed: {}", e))
        })?;

        let resp_payload = postcard::to_stdvec(&resp).map_err(|e| {
            error!(error = %e, "Failed to serialize AppendEntriesResponse");
            Status::internal(format!("Serialization failed: {}", e))
        })?;

        Ok(Response::new(RaftMessage {
            payload: resp_payload,
        }))
    }

    async fn request_vote(
        &self,
        request: Request<RaftMessage>,
    ) -> std::result::Result<Response<RaftMessage>, Status> {
        let req_payload = request.into_inner().payload;

        let req: VoteRequest<RaftTypeConfig> = postcard::from_bytes(&req_payload).map_err(|e| {
            error!(error = %e, "Failed to deserialize VoteRequest");
            Status::invalid_argument(format!("Deserialization failed: {}", e))
        })?;

        debug!(vote = %req.vote, "Received RequestVote");

        let resp = self.handler.vote(req).await.map_err(|e| {
            error!(error = ?e, "RequestVote failed");
            Status::internal(format!("Raft operation failed: {}", e))
        })?;

        let resp_payload = postcard::to_stdvec(&resp).map_err(|e| {
            error!(error = %e, "Failed to serialize VoteResponse");
            Status::internal(format!("Serialization failed: {}", e))
        })?;

        Ok(Response::new(RaftMessage {
            payload: resp_payload,
        }))
    }

    async fn install_snapshot(
        &self,
        request: Request<RaftMessage>,
    ) -> std::result::Result<Response<RaftMessage>, Status> {
        let req_payload = request.into_inner().payload;

        let req: InstallSnapshotRequest<RaftTypeConfig> = postcard::from_bytes(&req_payload)
            .map_err(|e| {
                error!(error = %e, "Failed to deserialize InstallSnapshotRequest");
                Status::invalid_argument(format!("Deserialization failed: {}", e))
            })?;

        debug!(vote = %req.vote, offset = req.offset, done = req.done, "Received InstallSnapshot");

        let resp = self.handler.install_snapshot(req).await.map_err(|e| {
            error!(error = ?e, "InstallSnapshot failed");
            Status::internal(format!("Raft operation failed: {}", e))
        })?;

        let resp_payload = postcard::to_stdvec(&resp).map_err(|e| {
            error!(error = %e, "Failed to serialize InstallSnapshotResponse");
            Status::internal(format!("Serialization failed: {}", e))
        })?;

        Ok(Response::new(RaftMessage {
            payload: resp_payload,
        }))
    }
}

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Server bind address
    pub addr: String,
    /// Maximum message size (bytes)
    pub max_message_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:50051".to_string(),
            max_message_size: 64 * 1024 * 1024, // 64MB
        }
    }
}

/// Start the gRPC server.
pub async fn start_server<H>(config: ServerConfig, handler: Arc<H>) -> Result<()>
where
    H: RaftServiceHandler,
{
    start_server_with_shutdown(config, handler, pending()).await
}

/// Start the gRPC server with a shutdown signal.
pub async fn start_server_with_shutdown<H, F>(
    config: ServerConfig,
    handler: Arc<H>,
    shutdown: F,
) -> Result<()>
where
    H: RaftServiceHandler,
    F: Future<Output = ()> + Send + 'static,
{
    let server = RaftGrpcServer::new(handler);
    let service = RaftServiceServer::new(server)
        .max_decoding_message_size(config.max_message_size)
        .max_encoding_message_size(config.max_message_size);

    let addr = config
        .addr
        .parse()
        .map_err(|e| Error::Network(format!("Invalid server address {}: {}", config.addr, e)))?;

    tracing::info!(addr = %addr, "Starting gRPC server");

    tonic::transport::Server::builder()
        .add_service(service)
        .serve_with_shutdown(addr, shutdown)
        .await
        .map_err(|e| Error::Network(format!("Server failed: {}", e)))?;

    Ok(())
}
