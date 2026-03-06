use openraft::errors::RaftError;
use openraft::network::{RPCOption, RaftNetworkFactory, RaftNetworkV2};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::storage::SnapshotMeta;
use openraft::{Snapshot, Vote};
use openraft_surrealkv::network::GrpcRaftNetworkFactory;
use openraft_surrealkv::network::server::{
    RaftServiceHandler, ServerConfig, start_server, start_server_with_shutdown,
};
use openraft_surrealkv::types::RaftTypeConfig;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};

const ADDR_NODE1: &str = "127.0.0.1:50051";
const ADDR_NODE2: &str = "127.0.0.1:50052";
const ADDR_SHUTDOWN: &str = "127.0.0.1:50053";
const ADDR_STRESS_ONE: &str = "127.0.0.1:50054";
const ADDR_STRESS_TWO_A: &str = "127.0.0.1:50055";
const ADDR_STRESS_TWO_B: &str = "127.0.0.1:50056";

const WAIT_SERVER_START_LONG_MS: u64 = 300;
const WAIT_SERVER_START_SHORT_MS: u64 = 250;
const REUSE_STRESS_ROUNDS: u64 = 50;

// Mock RaftServiceHandler implementation for the server
#[derive(Clone)]
struct MockHandler;

#[async_trait::async_trait]
impl RaftServiceHandler for MockHandler {
    async fn append_entries(
        &self,
        req: AppendEntriesRequest<RaftTypeConfig>,
    ) -> Result<AppendEntriesResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>> {
        let _ = req;
        Ok(AppendEntriesResponse::Success)
    }

    async fn vote(
        &self,
        req: VoteRequest<RaftTypeConfig>,
    ) -> Result<VoteResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>> {
        Ok(VoteResponse {
            vote: req.vote,
            vote_granted: true,
            last_log_id: req.last_log_id,
        })
    }

    async fn install_snapshot(
        &self,
        req: InstallSnapshotRequest<RaftTypeConfig>,
    ) -> Result<InstallSnapshotResponse<RaftTypeConfig>, RaftError<RaftTypeConfig>> {
        Ok(InstallSnapshotResponse { vote: req.vote })
    }
}

fn make_vote(term: u64, node_id: u64) -> Vote<RaftTypeConfig> {
    Vote::new(term, node_id)
}

fn make_append_req(term: u64, node_id: u64) -> AppendEntriesRequest<RaftTypeConfig> {
    AppendEntriesRequest {
        vote: make_vote(term, node_id),
        prev_log_id: None,
        entries: vec![],
        leader_commit: None,
    }
}

fn make_vote_req(term: u64, node_id: u64) -> VoteRequest<RaftTypeConfig> {
    VoteRequest {
        vote: make_vote(term, node_id),
        last_log_id: None,
    }
}

fn make_snapshot(bytes: &[u8]) -> Snapshot<RaftTypeConfig> {
    Snapshot {
        meta: SnapshotMeta::default(),
        snapshot: Cursor::new(bytes.to_vec()),
    }
}

fn rpc_option() -> RPCOption {
    RPCOption::new(Duration::from_secs(1))
}

fn make_factory(targets: &[(u64, &str)]) -> GrpcRaftNetworkFactory {
    let mut addresses = HashMap::new();
    for (id, addr) in targets {
        addresses.insert(*id, (*addr).to_string());
    }
    GrpcRaftNetworkFactory::with_static_addresses(addresses)
}

fn spawn_mock_server(addr: &str) {
    let config = ServerConfig {
        addr: addr.to_string(),
        ..Default::default()
    };
    tokio::spawn(async move {
        start_server(config, Arc::new(MockHandler)).await.unwrap();
    });
}

#[tokio::test]
async fn test_network_integration() -> anyhow::Result<()> {
    // Start server 1
    let addr1 = ADDR_NODE1;
    spawn_mock_server(addr1);

    // Start server 2
    let addr2 = ADDR_NODE2;
    spawn_mock_server(addr2);

    // Wait for servers to start
    sleep(Duration::from_millis(WAIT_SERVER_START_LONG_MS)).await;

    // Setup client factory
    let mut factory = make_factory(&[(1, addr1), (2, addr2)]);

    // Test AppendEntries to Node 1
    let mut client1 = factory
        .new_client(1, &openraft::BasicNode::new(addr1))
        .await;
    let append_req = make_append_req(1, 1);
    let append_resp = client1.append_entries(append_req, rpc_option()).await?;
    assert!(matches!(append_resp, AppendEntriesResponse::Success));

    // Test RequestVote to Node 2
    let mut client2 = factory
        .new_client(2, &openraft::BasicNode::new(addr2))
        .await;
    let vote_req = make_vote_req(1, 2);
    let vote_resp = client2.vote(vote_req, rpc_option()).await?;
    assert!(vote_resp.vote_granted);

    // Test FullSnapshot path (internally chunked InstallSnapshot RPC)
    let snapshot = make_snapshot(&[1, 2, 3, 4]);
    let snap_resp = client1
        .full_snapshot(
            make_vote(2, 1),
            snapshot,
            std::future::pending(),
            rpc_option(),
        )
        .await?;
    assert_eq!(snap_resp.vote, make_vote(2, 1));

    // One connection per target should be cached and reused.
    assert_eq!(factory.cached_connection_count().await, 2);
    Ok(())
}

#[tokio::test]
async fn test_graceful_shutdown() -> anyhow::Result<()> {
    let addr = ADDR_SHUTDOWN;
    let config = ServerConfig {
        addr: addr.to_string(),
        ..Default::default()
    };

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        start_server_with_shutdown(config, Arc::new(MockHandler), async move {
            let _ = shutdown_rx.await;
        })
        .await
    });

    sleep(Duration::from_millis(WAIT_SERVER_START_SHORT_MS)).await;

    let mut factory = make_factory(&[(1, addr)]);
    let mut client = factory.new_client(1, &openraft::BasicNode::new(addr)).await;

    let append_req = make_append_req(3, 1);
    client.append_entries(append_req, rpc_option()).await?;

    shutdown_tx.send(()).ok();
    timeout(Duration::from_secs(2), join).await???;

    Ok(())
}

#[tokio::test]
async fn test_connection_reuse_stress() -> anyhow::Result<()> {
    let addr = ADDR_STRESS_ONE;
    spawn_mock_server(addr);

    sleep(Duration::from_millis(WAIT_SERVER_START_SHORT_MS)).await;

    let mut factory = make_factory(&[(1, addr)]);

    let mut client = factory.new_client(1, &openraft::BasicNode::new(addr)).await;

    // Warm up one connection, then assert it is reused across repeated RPCs.
    let warmup = make_append_req(10, 1);
    client.append_entries(warmup, rpc_option()).await?;

    let baseline = factory.cached_connection_count().await;
    assert_eq!(baseline, 1);

    for i in 0..REUSE_STRESS_ROUNDS {
        let append_req = make_append_req(11 + i, 1);
        let append_resp = client.append_entries(append_req, rpc_option()).await?;
        assert!(matches!(append_resp, AppendEntriesResponse::Success));

        let vote_req = make_vote_req(11 + i, 1);
        let vote_resp = client.vote(vote_req, rpc_option()).await?;
        assert!(vote_resp.vote_granted);

        let snapshot = make_snapshot(&[1, 2, 3]);
        let _ = client
            .full_snapshot(
                make_vote(11 + i, 1),
                snapshot,
                std::future::pending(),
                rpc_option(),
            )
            .await?;

        assert_eq!(factory.cached_connection_count().await, baseline);
    }

    Ok(())
}

#[tokio::test]
async fn test_connection_reuse_stress_two_targets() -> anyhow::Result<()> {
    let addr1 = ADDR_STRESS_TWO_A;
    spawn_mock_server(addr1);

    let addr2 = ADDR_STRESS_TWO_B;
    spawn_mock_server(addr2);

    sleep(Duration::from_millis(WAIT_SERVER_START_LONG_MS)).await;

    let mut factory = make_factory(&[(1, addr1), (2, addr2)]);

    let mut client1 = factory
        .new_client(1, &openraft::BasicNode::new(addr1))
        .await;
    let mut client2 = factory
        .new_client(2, &openraft::BasicNode::new(addr2))
        .await;

    // Warm up both targets so the pool should stabilize at 2 cached connections.
    let warmup = make_append_req(30, 1);
    client1.append_entries(warmup.clone(), rpc_option()).await?;
    client2.append_entries(warmup, rpc_option()).await?;

    let baseline = factory.cached_connection_count().await;
    assert_eq!(baseline, 2);

    for i in 0..REUSE_STRESS_ROUNDS {
        let append1 = make_append_req(31 + i, 1);
        let append2 = make_append_req(31 + i, 2);

        let r1 = client1.append_entries(append1, rpc_option()).await?;
        let r2 = client2.append_entries(append2, rpc_option()).await?;
        assert!(matches!(r1, AppendEntriesResponse::Success));
        assert!(matches!(r2, AppendEntriesResponse::Success));

        let v1 = client1.vote(make_vote_req(31 + i, 1), rpc_option()).await?;
        let v2 = client2.vote(make_vote_req(31 + i, 2), rpc_option()).await?;
        assert!(v1.vote_granted);
        assert!(v2.vote_granted);

        let snap = make_snapshot(&[9, 8, 7, 6]);
        let _ = client1
            .full_snapshot(
                make_vote(31 + i, 1),
                snap.clone(),
                std::future::pending(),
                rpc_option(),
            )
            .await?;
        let _ = client2
            .full_snapshot(
                make_vote(31 + i, 2),
                snap,
                std::future::pending(),
                rpc_option(),
            )
            .await?;

        assert_eq!(factory.cached_connection_count().await, baseline);
    }

    Ok(())
}
