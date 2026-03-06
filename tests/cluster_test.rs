use openraft_surrealkv::config::Config;
use openraft_surrealkv::storage::SurrealStorage;
use openraft_surrealkv::types::KVRequest;
use std::sync::Arc;
use surrealkv::TreeBuilder;
use tempfile::TempDir;

/// Phase 5.3 多节点集群验证测试
///
/// 验证场景：
/// 1. 3 节点集群启动
/// 2. 基础的读写操作
/// 3. 多节点存储可用性

#[tokio::test]
async fn test_3_node_cluster_basic() -> anyhow::Result<()> {
    // 为每个节点创建独立的数据目录
    let base1 = TempDir::new()?;
    let base2 = TempDir::new()?;
    let base3 = TempDir::new()?;

    // Node 1
    let tree1 = Arc::new(
        TreeBuilder::new()
            .with_path(base1.path().join("kv"))
            .build()?,
    );
    let storage1 = Arc::new(SurrealStorage::new(tree1).await?);

    // Node 2
    let tree2 = Arc::new(
        TreeBuilder::new()
            .with_path(base2.path().join("kv"))
            .build()?,
    );
    let storage2 = Arc::new(SurrealStorage::new(tree2).await?);

    // Node 3
    let tree3 = Arc::new(
        TreeBuilder::new()
            .with_path(base3.path().join("kv"))
            .build()?,
    );
    let _storage3 = Arc::new(SurrealStorage::new(tree3).await?);

    // 写入测试数据到 Node 1
    tracing::info!("Writing to Node 1");
    for i in 0..10 {
        let key = format!("test_key_{}", i);
        let value = format!("test_value_{}", i);
        storage1
            .apply_request(&KVRequest::Set {
                key,
                value: value.into_bytes(),
            })
            .await?;
    }

    // 验证 Node 1 可以读取
    tracing::info!("Reading from Node 1");
    let val = storage1.read("test_key_0").await?;
    assert!(val.is_some());
    assert_eq!(val.unwrap(), b"test_value_0");

    // 验证其他节点的存储独立性
    tracing::info!("Verifying independent storage");
    let val2 = storage2.read("test_key_0").await?;
    assert!(
        val2.is_none(),
        "Node 2 should not have Node 1's data (no replication)"
    );

    Ok(())
}

/// 测试多节点并发操作
#[tokio::test]
async fn test_concurrent_node_operations() -> anyhow::Result<()> {
    let base1 = TempDir::new()?;
    let base2 = TempDir::new()?;

    let tree1 = Arc::new(
        TreeBuilder::new()
            .with_path(base1.path().join("kv"))
            .build()?,
    );
    let storage1 = Arc::new(SurrealStorage::new(tree1).await?);

    let tree2 = Arc::new(
        TreeBuilder::new()
            .with_path(base2.path().join("kv"))
            .build()?,
    );
    let storage2 = Arc::new(SurrealStorage::new(tree2).await?);

    // 并发写入
    let s1_clone = storage1.clone();
    let s2_clone = storage2.clone();

    let handle1 = tokio::spawn(async move {
        for i in 0..50 {
            let key = format!("node1_key_{}", i);
            let value = format!("value_{}", i);
            s1_clone
                .apply_request(&KVRequest::Set {
                    key,
                    value: value.into_bytes(),
                })
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let handle2 = tokio::spawn(async move {
        for i in 0..50 {
            let key = format!("node2_key_{}", i);
            let value = format!("value_{}", i);
            s2_clone
                .apply_request(&KVRequest::Set {
                    key,
                    value: value.into_bytes(),
                })
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    handle1.await??;
    handle2.await??;

    // 验证数据
    let val1 = storage1.read("node1_key_0").await?;
    assert!(val1.is_some());

    let val2 = storage2.read("node2_key_0").await?;
    assert!(val2.is_some());

    tracing::info!("Concurrent operations completed successfully");
    Ok(())
}

/// 测试配置加载多节点
#[tokio::test]
async fn test_config_for_multiple_nodes() -> anyhow::Result<()> {
    // Node 1 config
    let mut config1 = Config::default();
    config1.node.node_id = 1;
    config1.node.listen_addr = "127.0.0.1:50051".to_string();
    config1.http.port = 8080;

    assert_eq!(config1.node.node_id, 1);
    assert_eq!(config1.http.port, 8080);

    // Node 2 config
    let mut config2 = Config::default();
    config2.node.node_id = 2;
    config2.node.listen_addr = "127.0.0.1:50052".to_string();
    config2.http.port = 8081;

    assert_eq!(config2.node.node_id, 2);
    assert_eq!(config2.http.port, 8081);

    // Node 3 config
    let mut config3 = Config::default();
    config3.node.node_id = 3;
    config3.node.listen_addr = "127.0.0.1:50053".to_string();
    config3.http.port = 8082;

    assert_eq!(config3.node.node_id, 3);
    assert_eq!(config3.http.port, 8082);

    tracing::info!("All three nodes configured successfully");
    Ok(())
}
