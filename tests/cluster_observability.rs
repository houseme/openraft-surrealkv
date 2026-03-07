#![cfg(feature = "integration-cluster")]

mod common;

use common::cluster_harness::{
    format_status, nodes_of, shutdown_all, snapshot_status, start_three_nodes,
    wait_stable_single_leader,
};
use openraft_surrealkv::types::KVRequest;
use serial_test::serial;
use std::time::Duration;

#[tokio::test]
#[serial]
async fn test_three_node_election_observability() -> anyhow::Result<()> {
    let (runtimes, c1) = start_three_nodes().await?;
    let nodes = nodes_of(&runtimes);

    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = nodes[0].initialize_cluster(c1.cluster_members()).await;

    let election = wait_stable_single_leader(&nodes, Duration::from_secs(12)).await;
    if let Err(e) = election {
        shutdown_all(runtimes).await;
        return Err(e);
    }

    shutdown_all(runtimes).await;
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_three_node_replication_visibility() -> anyhow::Result<()> {
    let (runtimes, c1) = start_three_nodes().await?;
    let nodes = nodes_of(&runtimes);

    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = nodes[0].initialize_cluster(c1.cluster_members()).await;

    let (leader_id, _) = match wait_stable_single_leader(&nodes, Duration::from_secs(12)).await {
        Ok(x) => x,
        Err(e) => {
            shutdown_all(runtimes).await;
            return Err(e);
        }
    };

    let leader = match leader_id {
        1 => nodes[0].clone(),
        2 => nodes[1].clone(),
        3 => nodes[2].clone(),
        _ => {
            shutdown_all(runtimes).await;
            anyhow::bail!("unexpected leader id: {}", leader_id)
        }
    };

    let key = "replication_key".to_string();
    let val = b"replication_value".to_vec();
    leader
        .client_write(KVRequest::Set {
            key: key.clone(),
            value: val.clone(),
        })
        .await?;

    let repl_deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let mut replicated = false;
    while tokio::time::Instant::now() < repl_deadline {
        let v1 = nodes[0].read_key(&key).await?;
        let v2 = nodes[1].read_key(&key).await?;
        let v3 = nodes[2].read_key(&key).await?;

        if v1.as_deref() == Some(val.as_slice())
            && v2.as_deref() == Some(val.as_slice())
            && v3.as_deref() == Some(val.as_slice())
        {
            replicated = true;
            break;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    if !replicated {
        let status = snapshot_status(&nodes).await;
        shutdown_all(runtimes).await;
        anyhow::bail!(
            "expected value to replicate to all 3 nodes; status: {}",
            format_status(&status)
        );
    }

    shutdown_all(runtimes).await;
    Ok(())
}
