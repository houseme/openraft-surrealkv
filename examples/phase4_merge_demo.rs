//! Phase 4 demo: storage initialization with automatic background merge and crash recovery.
//!
//! This example demonstrates:
//! 1. Initializing `SurrealStorage` with automatic merge policy enabled
//! 2. Running crash-recovery checks during startup
//! 3. Building snapshots that may trigger merge jobs automatically

use openraft_surrealkv::error::Result;
use openraft_surrealkv::merge::DeltaMergePolicy;
use openraft_surrealkv::storage::SurrealStorage;
use openraft_surrealkv::types::KVRequest;
use std::sync::Arc;
use surrealkv::TreeBuilder;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging for demo visibility.
    tracing_subscriber::fmt::init();

    println!("=== Phase 4 Automatic Background Merge Demo ===\n");

    // 1. Create the SurrealKV engine.
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path("target/example_phase4_data".into())
            .build()?,
    );

    // 2. Create storage with the default merge policy.
    let storage = SurrealStorage::new(tree)
        .await?
        .with_default_merge("node_1"); // Default policy: 5 deltas / 300MB / 24h

    println!("✅ Storage initialized with automatic background merge enabled");

    // 3. Run startup crash-recovery checks.
    storage.recover_merge_state().await?;
    println!("✅ Crash-recovery check completed\n");

    // 4. Simulate applied requests and append corresponding Raft log entries.
    println!("📝 Simulating 10 applied requests...");
    for i in 1..=10 {
        let req = KVRequest::Set {
            key: format!("key_{}", i),
            value: vec![i as u8; 100],
        };
        storage.apply_request(&req).await?;
        storage.append_log_entry(1, i, &req).await?;
    }
    println!("✅ 10 requests applied\n");

    // 5. Build a snapshot (merge eligibility is checked automatically).
    println!("📸 Building snapshot...");
    let snapshot = storage.build_snapshot_auto(1).await?;
    println!(
        "✅ Snapshot build completed, snapshot_id={}",
        snapshot.meta.snapshot_id
    );
    println!(
        "   (a background merge job is scheduled automatically when policy thresholds are met)\n"
    );

    // 6. Demonstrate a custom merge policy.
    println!("=== Custom Merge Policy Demo ===");
    let custom_policy = DeltaMergePolicy {
        max_chain_length: 3,                    // Trigger when 3 deltas accumulate
        max_delta_bytes: 100 * 1024 * 1024,     // Trigger at 100MB cumulative delta size
        checkpoint_interval_secs: 12 * 60 * 60, // Trigger every 12 hours
    };

    let tree2 = Arc::new(
        TreeBuilder::new()
            .with_path("target/example_phase4_custom".into())
            .build()?,
    );

    let _storage_custom = SurrealStorage::new(tree2)
        .await?
        .with_merge_policy(custom_policy, "node_2");
    println!("✅ Storage created with custom merge policy\n");

    println!("=== Demo completed ===");
    println!("\n💡 Key capabilities:");
    println!("   • Background merge is fully asynchronous and does not block foreground work");
    println!("   • Three-dimensional policy decisions (chain length > bytes > time)");
    println!("   • Crash recovery detects incomplete merge jobs and retries them automatically");
    println!("   • Prometheus metrics track merge outcomes and runtime status");

    Ok(())
}
