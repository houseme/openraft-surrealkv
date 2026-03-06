//! Phase 4 示例：带自动后台合并的存储初始化与崩溃恢复
//!
//! 此示例演示：
//! 1. 启用自动后台合并的 SurrealStorage 初始化
//! 2. 崩溃恢复流程（启动时自动检查未完成任务）
//! 3. 快照构建后自动触发合并

use openraft_surrealkv::error::Result;
use openraft_surrealkv::merge::DeltaMergePolicy;
use openraft_surrealkv::storage::SurrealStorage;
use openraft_surrealkv::types::KVRequest;
use std::sync::Arc;
use surrealkv::TreeBuilder;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt::init();

    println!("=== Phase 4 自动后台合并示例 ===\n");

    // 1. 创建 SurrealKV 引擎
    let tree = Arc::new(
        TreeBuilder::new()
            .with_path("target/example_phase4_data".into())
            .build()?,
    );

    // 2. 创建 Storage 并启用默认合并策略
    let storage = SurrealStorage::new(tree)
        .await?
        .with_default_merge("node_1"); // 使用默认策略：5 delta / 300MB / 24h

    println!("✅ Storage 初始化完成，已启用自动后台合并");

    // 3. 启动时执行崩溃恢复检查
    storage.recover_merge_state().await?;
    println!("✅ 崩溃恢复检查完成\n");

    // 4. 模拟应用请求并记录日志
    println!("📝 模拟应用 10 个请求...");
    for i in 1..=10 {
        let req = KVRequest::Set {
            key: format!("key_{}", i),
            value: vec![i as u8; 100],
        };
        storage.apply_request(&req).await?;
        storage.append_log_entry(1, i, &req).await?;
    }
    println!("✅ 10 个请求已应用\n");

    // 5. 构建快照（会自动检查是否触发合并）
    println!("📸 构建快照...");
    let snapshot = storage.build_snapshot_auto(1).await?;
    println!("✅ 快照构建完成，snapshot_id={}", snapshot.meta.snapshot_id);
    println!("   (如果满足合并策略，后台任务已自动启动)\n");

    // 6. 自定义合并策略示例
    println!("=== 自定义合并策略示例 ===");
    let custom_policy = DeltaMergePolicy {
        max_chain_length: 3,                    // 3 个 delta 触发
        max_delta_bytes: 100 * 1024 * 1024,     // 100MB 触发
        checkpoint_interval_secs: 12 * 60 * 60, // 12 小时触发
    };

    let tree2 = Arc::new(
        TreeBuilder::new()
            .with_path("target/example_phase4_custom".into())
            .build()?,
    );

    let _storage_custom = SurrealStorage::new(tree2)
        .await?
        .with_merge_policy(custom_policy, "node_2");
    println!("✅ 已创建带自定义策略的 Storage\n");

    println!("=== 示例运行完成 ===");
    println!("\n💡 关键特性：");
    println!("   • 后台合并完全异步，不阻塞主线程");
    println!("   • 三维策略自动决策（链长 > 字节 > 时间）");
    println!("   • 崩溃恢复自动识别未完成任务并重试");
    println!("   • Prometheus 指标自动记录合并状态");

    Ok(())
}
