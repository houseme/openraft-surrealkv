use openraft_surrealkv::storage::SurrealStorage;
use openraft_surrealkv::types::KVRequest;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use surrealkv::TreeBuilder;
use tempfile::TempDir;

/// Phase 5.3 stress test: verify sustained 1000+ QPS write throughput.

#[tokio::test]
async fn test_1000_qps_throughput() -> anyhow::Result<()> {
    let base = TempDir::new()?;
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("kv"))
            .build()?,
    );
    let storage = Arc::new(SurrealStorage::new(tree).await?);

    // Aggregate performance counters collected from all worker tasks.
    let total_ops = Arc::new(AtomicU64::new(0));
    let failed_ops = Arc::new(AtomicU64::new(0));

    let start = Instant::now();
    let duration_secs = 10; // Run load for 10 seconds.

    // Launch 50 workers; each worker attempts 20 writes per loop cycle.
    let mut handles = vec![];

    for task_id in 0..50 {
        let storage_clone = storage.clone();
        let total = total_ops.clone();
        let failed = failed_ops.clone();

        let handle = tokio::spawn(async move {
            let mut local_ops = 0u64;
            let mut local_failed = 0u64;

            while start.elapsed().as_secs() < duration_secs {
                for i in 0..20 {
                    let key = format!("stress_test_{}_{}", task_id, i);
                    let value = format!("value_{}", i);

                    match storage_clone
                        .apply_request(&KVRequest::Set {
                            key,
                            value: value.into_bytes(),
                        })
                        .await
                    {
                        Ok(_) => local_ops += 1,
                        Err(_) => local_failed += 1,
                    }
                }

                // Yield periodically so workers share executor time fairly.
                tokio::task::yield_now().await;
            }

            total.fetch_add(local_ops, Ordering::Relaxed);
            failed.fetch_add(local_failed, Ordering::Relaxed);
        });

        handles.push(handle);
    }

    // Wait for all worker tasks to finish and flush counters.
    for handle in handles {
        handle.await?;
    }

    let elapsed = start.elapsed();
    let total_ops_count = total_ops.load(Ordering::Relaxed);
    let failed_count = failed_ops.load(Ordering::Relaxed);
    let qps = total_ops_count as f64 / elapsed.as_secs_f64();

    tracing::info!(
        total_ops = total_ops_count,
        failed_ops = failed_count,
        elapsed_secs = elapsed.as_secs_f64(),
        qps = qps,
        "Stress test completed"
    );

    // Throughput target: at least 1000 QPS.
    assert!(qps >= 1000.0, "QPS {} should be >= 1000", qps);

    // Error budget: failed operations must remain below 1%.
    let failure_rate = failed_count as f64 / total_ops_count as f64;
    assert!(
        failure_rate < 0.01,
        "Failure rate {} should be < 1%",
        failure_rate * 100.0
    );

    Ok(())
}

/// Verify mixed concurrent read/write throughput under balanced workload.
#[tokio::test]
async fn test_mixed_read_write_operations() -> anyhow::Result<()> {
    let base = TempDir::new()?;
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path(base.path().join("kv"))
            .build()?,
    );
    let storage = Arc::new(SurrealStorage::new(tree).await?);

    // Seed baseline keys so read workers hit existing data.
    for i in 0..100 {
        storage
            .apply_request(&KVRequest::Set {
                key: format!("key_{}", i),
                value: format!("value_{}", i).into_bytes(),
            })
            .await?;
    }

    let total_ops = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    // 25 read workers + 25 write workers = 50 concurrent workers.
    let mut handles = vec![];

    // Read workers.
    for task_id in 0..25 {
        let storage_clone = storage.clone();
        let total = total_ops.clone();

        let handle = tokio::spawn(async move {
            let mut local_ops = 0u64;

            while start.elapsed().as_secs() < 5 {
                for i in 0..10 {
                    let key = format!("key_{}", (task_id * 10 + i) % 100);
                    if storage_clone.read(&key).await.is_ok() {
                        local_ops += 1;
                    }
                }
            }

            total.fetch_add(local_ops, Ordering::Relaxed);
        });

        handles.push(handle);
    }

    // Write workers.
    for task_id in 0..25 {
        let storage_clone = storage.clone();
        let total = total_ops.clone();

        let handle = tokio::spawn(async move {
            let mut local_ops = 0u64;
            let mut counter = 0u64;

            while start.elapsed().as_secs() < 5 {
                for i in 0..10 {
                    counter += 1;
                    if storage_clone
                        .apply_request(&KVRequest::Set {
                            key: format!("write_task_{}_{}_{}", task_id, i, counter),
                            value: "test_value".as_bytes().to_vec(),
                        })
                        .await
                        .is_ok()
                    {
                        local_ops += 1;
                    }
                }
            }

            total.fetch_add(local_ops, Ordering::Relaxed);
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.await?;
    }

    let elapsed = start.elapsed();
    let total_ops_count = total_ops.load(Ordering::Relaxed);
    let qps = total_ops_count as f64 / elapsed.as_secs_f64();

    tracing::info!(
        total_ops = total_ops_count,
        elapsed_secs = elapsed.as_secs_f64(),
        qps = qps,
        "Mixed R/W test completed"
    );

    // Mixed R/W workload should stay above the baseline target.
    assert!(qps > 500.0, "QPS {} should be > 500", qps);

    Ok(())
}
