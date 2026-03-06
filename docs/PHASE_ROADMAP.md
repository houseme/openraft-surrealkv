# SurrealKV + OpenRaft 分布式一致性 KV 系统 - 完整 Roadmap

## 文档目的

本文档概述了从 **Phase 0（基础框架）** 到 **Phase 5（生产优化）** 的完整 5 阶段开发计划，包括：

- 各阶段的目标与核心功能
- 需要新建/修改的关键文件
- 验证点与交付物
- 工作量与风险评估

**状态**: Phase 0 ✅ 完成，Phase 3 🔄 v2 实现中（其余阶段按计划推进）

---

## Phase 0: 基础框架 ✅ 完成

**预计工作量**: 80h | **实际**: ~80h | **状态**: ✅

### 目标

建立 OpenRaft 类型系统与 SurrealKV 存储层基础，实现单节点基础功能。

### 核心任务 ✅

- [x] TypeConfig 定义（Request/Response/NodeId/SnapshotData）
- [x] SurrealStorage 框架（RaftLogStorage + RaftStateMachine trait）
- [x] MetadataManager（投票、应用状态、快照元数据）
- [x] RaftSnapshotBuilder 框架
- [x] 错误类型系统 & 单元测试

### 文件清单 ✅

- ✅ `src/lib.rs` - 模块导出与文档
- ✅ `src/error.rs` - 统一错误类型
- ✅ `src/types.rs` - OpenRaft TypeConfig 定义
- ✅ `src/state.rs` - 元数据管理
- ✅ `src/snapshot.rs` - 快照框架
- ✅ `src/storage.rs` - 存储实现
- ✅ `src/network.rs` - 网络层占位符
- ✅ `src/merge.rs` - 合并策略占位符

### 验证点 ✅

- [x] cargo build --lib 通过
- [x] 30+ 单元测试（75%+ 覆盖率）
- [x] 存储键前缀设计完成
- [x] 类型安全验证（Rust 编译）

### 交付物 ✅

- ✅ 完整的模块框架（~1100 LOC）
- ✅ 单元测试套件
- ✅ 详细代码注释
- ✅ IMPLEMENTATION_STATUS.md
- ✅ 本 Roadmap 文档

---

## Phase 1: 网络层与 gRPC 实现

**预计工作量**: 70h | **当前**: 设计完成 | **状态**: 🔄 就绪启动

### 目标

实现 Tonic gRPC 网络通信层，支持 OpenRaft 网络 RPC（AppendEntries, RequestVote, InstallSnapshot）。

### 前置条件

- ✅ Phase 0 完成
- ✅ proto/raft.proto 已定义
- ✅ build.rs 已配置 tonic-prost-build

### 核心任务

1. **RaftNetworkFactory** 实现
    - 管理到其他节点的 gRPC 连接
    - 连接缓存与重用
    - 超时配置

2. **RaftNetworkV2** 实现
    - AppendEntries RPC -> `append_entries()`
    - Vote RPC -> `vote()`
    - FullSnapshot RPC（基础分块版）-> `full_snapshot()`

3. **Tonic gRPC 服务器**
    - RaftService 实现（三个 RPC 处理器）
    - postcard 序列化/反序列化中间件
    - 优雅关闭处理

4. **网络特性**
    - HTTP/2 多路复用（Tonic 内置）
    - 重试与超时机制
    - 连接池管理

### 文件规划

- `src/network.rs` - 修改：完整 RaftNetworkFactory + RaftNetworkV2
    - 新增子模块：`network/client.rs` (gRPC 客户端)
    - 新增子模块：`network/server.rs` (gRPC 服务器)
- `src/network/client.rs` - NEW：gRPC 客户端实现
- `src/network/server.rs` - NEW：RaftService 处理器
- `src/main.rs` - NEW：应用入口（暂时放在 Phase 5，这里可创建空占位符）
- `tests/network_integration.rs` - NEW：两节点 RPC 测试

### 关键 API（OpenRaft trait, 0.10）

```rust
// RaftNetworkFactory trait
pub trait RaftNetworkFactory<C: RaftTypeConfig> {
    type Network: RaftNetworkV2<C>;
    async fn new_client(
        &mut self,
        target: C::NodeId,
        node: &C::Node,
    ) -> Self::Network;
}

// RaftNetworkV2 trait (核心方法)
pub trait RaftNetworkV2<C: RaftTypeConfig> {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>>;

    async fn vote(
        &mut self,
        rpc: VoteRequest<C>,
        option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>>;

    async fn full_snapshot(
        &mut self,
        vote: Vote<C>,
        snapshot: Snapshot<C>,
        cancel: impl Future<Output=ReplicationClosed> + Send + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>>;
}
```

### 验证点

- [ ] cargo build 通过
- [ ] 两节点 gRPC 通信集成测试通过
- [ ] RPC 延迟 <100ms（本地）
- [ ] HTTP/2 连接复用验证
- [ ] graceful shutdown 测试

### 交付物

- 完整的 gRPC 网络层
- RaftNetworkFactory 连接管理
- RaftNetwork trait 实现
- 集成测试（2+ 节点）
- 网络性能基准

### 预计时间表

- 周 1: RaftNetworkFactory + RaftNetworkV2 框架 (20h)
- 周 2: gRPC 服务器与处理器 (25h)
- 周 3: 测试与优化 (25h)

---

## Phase 2: SurrealKV Checkpoint 深度集成

**预计工作量**: 70h | **依赖**: Phase 0 + Phase 1 | **状态**: 📋 设计完成

### 目标

实现零拷贝快速快照、秒级原子恢复、Checkpoint 与 LogId 对齐。

### 前置条件

- ✅ Phase 0 + Phase 1 完成
- ✅ SurrealKV checkpoint API 理解
    - `create_checkpoint(dir)` → 硬链接零拷贝
    - `restore_from_checkpoint(dir)` → 原子恢复

### 核心任务

1. **Checkpoint 创建**
    - `create_checkpoint(log_id)` 方法
    - 生成 CheckpointMetadata（index, term, seq, timestamp）
    - SurrealKV 序列号与 Raft LogId 对齐验证

2. **Checkpoint 恢复**
    - `restore_from_checkpoint(metadata)` 方法
    - 原子性恢复流程
    - 一致性验证（checksums）

3. **快照打包与压缩**
    - 异步 walkdir 遍历 checkpoint 目录
    - tokio-tar 流式 tar 打包
    - async-compression (zstd) 压缩
    - 体积降低预期：4-6×

4. **一致性保证**
    - applied_log_id 匹配验证
    - sequence_number 校验
    - 快照安装前完整性检查

### 文件规划

- `src/snapshot.rs` - 修改：完整 RaftSnapshotBuilder
    - 新增子模块：`snapshot/checkpoint.rs` (Checkpoint 操作)
    - 新增子模块：`snapshot/metadata.rs` (Checkpoint 元数据)
    - 新增子模块：`snapshot/compression.rs` (zstd 压缩)
- `src/snapshot/checkpoint.rs` - NEW：checkpoint 创建/恢复
- `src/snapshot/compression.rs` - NEW：tar + zstd 压缩
- `src/storage.rs` - 修改：集成 checkpoint 到 RaftStateMachine
- `tests/snapshot_checkpoint.rs` - NEW：checkpoint 一致性测试

### 关键 API（OpenRaft trait 扩展）

```rust
impl RaftStateMachine for SurrealStorage {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>> {
        // 调用 create_checkpoint()
        // 压缩为 tar.zst
        // 返回 snapshot data
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotData,
        snapshot: Snapshot<TypeConfig>,
    ) -> Result<()> {
        // 解压 tar.zst
        // 调用 restore_from_checkpoint()
        // 验证一致性
    }
}
```

### 性能目标

- Checkpoint 创建：O(1) （硬链接）
- 压缩：<10s（300MB 快照，取决于 CPU）
- 传输：基础 (Phase 1.5 优化分块)
- 恢复：<2s（秒级原子操作）
- 体积：原始 1GB → 压缩 ~150MB (zstd)

### 验证点

- [ ] Checkpoint 创建与恢复时间测试
- [ ] 数据完整性验证（CRC/SHA）
- [ ] 应用状态与 Checkpoint 对齐
- [ ] 容错测试（中断恢复）
- [ ] 磁盘空间管理（自动清理旧 checkpoints）

### 交付物

- Checkpoint API 完整实现
- 快照压缩与恢复流程
- 一致性验证框架
- 性能基准报告（>100MB/s 传输）

### 预计时间表

- 周 1: Checkpoint 创建与恢复 (20h)
- 周 2: 压缩与打包优化 (25h)
- 周 3: 一致性验证与测试 (25h)

---

## Phase 3: 增量快照与 Delta 机制

**预计工作量**: 50h | **依赖**: Phase 2 | **状态**: 🔄 v2 schema 已实现（进行中）

### 目标

实现轻量级增量快照，基于已应用 Entry 的 Delta 格式。

### 前置条件

- ✅ Phase 2 完成（全量快照就位）

### 核心任务

1. **Delta 来源**
    - 来自已应用 Entry (try_get_log_entries)
    - 范围：last_snapshot_index..current_applied_index
    - 包含完整 OpenRaft Entry（term, index, payload）

2. **Delta 格式**
    - `postcard(Vec<Entry<TypeConfig>>)` 序列化
    - zstd 压缩
    - 存储在临时文件（路径记录在 SnapshotMetaState）

3. **自动策略选择**
    - `build_snapshot()` 判断：是否生成 Delta
    - 条件：距上次全量快照的 Entry 数量 > 阈值（~1000）
    - 或：时间 < 24h 且 Delta 不超过 5MB

4. **Delta 应用**
    - `install_snapshot()` / `install_snapshot_auto()` 支持两种格式
    - 全量快照：直接 restore_from_checkpoint()
    - Delta: 对当前状态机 re-apply() entries
    - 语义约束：仅 Delta 走 storage-level apply（`install_snapshot_auto()` -> `apply_delta_entries()`）
    - strict error：`base_index` 不匹配立即 fail-fast，不自动回退
    - strict error：未知/非法 snapshot payload 直接报错（不尝试隐式降级）

### 文件规划

- `src/snapshot.rs` - 修改：Delta 构建逻辑
    - 新增子模块：`snapshot/delta.rs` (Delta 操作)
- `src/snapshot/delta.rs` - NEW：Delta 序列化/应用
- `src/types.rs` - 修改：DeltaFormat, DeltaMetadata 类型
- `src/storage.rs` - 修改：RaftStateMachine 支持 Delta
- `tests/delta_snapshot.rs` - NEW：Delta 构建与应用测试

### 关键类型

```rust
#[derive(Serialize, Deserialize)]
pub enum SnapshotFormat {
    /// 全量快照 (tar.zst)
    Full { checkpoint_meta: CheckpointMetadata },
    /// 增量快照 v2 (metadata-only)
    Delta {
        base_index: u64,
        entry_count: u64,
    },
}

pub struct DeltaMetadata {
    pub format: SnapshotFormat,
    pub size_bytes: u64,
    pub created_at: u64,
    pub file_path: Option<String>,
    pub version: u16, // 当前写入版本: 2
}
```

> 兼容性说明：当前实现“读兼容 v1+v2，写仅 v2”，并通过 `decode_snapshot_payload()` 统一解码。

### 性能目标

- Delta 大小：<5MB（相对全量快照 <10%）
- Delta 构建：<1s（内存操作）
- Delta 应用：<2s（re-apply entries）

### 验证点

- [x] Delta 大小 <5MB 验证
- [x] Delta 重放一致性测试
- [x] 全量→Delta 自动切换逻辑
- [x] 版本兼容性测试（v1->v2 解码）
- [ ] 快照链完整性验证（Phase 4 合并链路联调）

### 交付物

- [x] Delta 构建与应用完整实现
- [x] 自动格式选择逻辑
- [x] 版本兼容性层（读 v1/v2，写 v2）
- [ ] 增量快照性能基准（待完善）

### 预计时间表

- 周 1: Delta 格式定义与序列化 (15h)
- 周 2: Delta 构建与应用逻辑 (20h)
- 周 3: 测试与优化 (15h)

---

## Phase 4: Hybrid 合并策略与后台任务

**预计工作量**: 90h | **依赖**: Phase 0-3 | **状态**: 📋 设计完成

### 目标

实现三维决策的自动快照合并，后台异步执行，确保无阻塞。

### 前置条件

- ✅ Phase 0-3 完成

### 核心任务

1. **三维决策机制**
    - **链长** (max_chain_length = 5): Delta 数量 ≥ 5 时
    - **累计字节** (max_delta_bytes = 300MB): 总大小 ≥ 300MB 时
    - **时间窗口** (checkpoint_interval = 24h): 距上次全量 ≥ 24h 时
    - **权重优先级**: 链长 > 字节大小 > 时间

2. **后台异步合并**
    - tokio::spawn 独立任务
    - 不阻塞主 Raft 线程
    - 可中断与重试

3. **合并执行流程**
    - 创建临时 SurrealKV Tree
    - 顺序 re-apply 所有 Delta 中的 entries
    - 对临时 Tree create_checkpoint()
    - 原子替换基线快照
    - 清空 delta_chain，更新 last_checkpoint_seq

4. **状态持久化**
    - SnapshotMetaState（Immediate txn）
    - 记录合并进度（防重复）
    - 原子操作保证一致性

5. **自动清理**
    - 删除过期 delta 文件
    - 清理临时合并目录
    - 保留 N 个历史快照（可配置）

6. **错误处理与 Fallback**
    - 合并失败自动回退全量快照
    - 重试机制（最多 3 次）
    - 日志与追踪记录

### 文件规划

- `src/merge.rs` - 完整实现
    - `merge/policy.rs` - 三维决策逻辑
    - `merge/executor.rs` - 合并执行
    - `merge/cleanup.rs` - 文件清理
- `src/merge/policy.rs` - NEW：决策引擎
- `src/merge/executor.rs` - NEW：合并任务
- `src/state.rs` - 修改：扩展持久化状态（合并进度）
- `src/metrics.rs` - NEW：Prometheus 指标
- `tests/merge_policy.rs` - NEW：决策逻辑测试
- `tests/merge_executor.rs` - NEW：合并执行测试

### 关键类型

```rust
pub struct DeltaMergePolicy {
    pub max_chain_length: usize,        // 5
    pub max_delta_bytes: u64,           // 300MB
    pub checkpoint_interval_secs: u64,  // 24h
}

pub enum MergeTrigger {
    ChainTooLong,
    SizeTooLarge,
    TimeWindowExpired,
}

pub struct MergeExecutor {
    engine: Arc<Engine>,
    policy: DeltaMergePolicy,
}

pub struct MergeMetrics {
    pub merge_duration_ms: Histogram,
    pub merge_success_rate: Counter,
    pub merge_failure_rate: Counter,
    pub checkpoint_size_bytes: Gauge,
}
```

### 性能目标

- 合并检测：<100ns (内存判断)
- 后台合并时间：1-5s（取决于 delta 大小）
- 主线程阻塞：0ms (完全异步)
- 清理开销：<1s

### Prometheus 指标

```
raft_snapshot_merge_duration_ms{node_id="1"} = 2500
raft_snapshot_merge_success_total{node_id="1"} = 42
raft_snapshot_merge_failed_total{node_id="1"} = 1
raft_snapshot_checkpoint_size_bytes{node_id="1"} = 1073741824
raft_snapshot_delta_chain_length{node_id="1"} = 3
raft_snapshot_delta_cumulative_mb{node_id="1"} = 156
```

### 验证点

- [ ] 三维决策触发条件单元测试
- [ ] 后台合并不影响主线程（压力测试: 1000 QPS）
- [ ] 原子状态切换验证（无中间状态可见）
- [ ] 过期文件自动清理验证
- [ ] 一致性校验（checksum 匹配 100%）
- [ ] 并发合并安全性测试
- [ ] 错误恢复与重试测试

### 交付物

- 后台合并任务框架
- 三维策略引擎
- 状态管理与原子操作
- 合并可观测性指标
- 完整的容错机制

### 预计时间表

- 周 1: 决策逻辑与架构设计 (25h)
- 周 2: 合并执行与状态管理 (35h)
- 周 3: 测试、指标与优化 (30h)

---

## Phase 5: 生产优化与完整集群部署

**预计工作量**: 80h | **依赖**: Phase 0-4 | **状态**: 📋 设计完成

### 目标

生产级优化、HTTP API、监控可观测性、三节点集群完整部署验证。

### 前置条件

- ✅ Phase 0-4 完成

### 核心任务

1. **RaftNode 应用框架**
    - RaftNode 结构：整合 OpenRaft + SurrealStorage + Network
    - Membership management：add_learner, change_membership
    - Client write 流程：client_write -> append -> apply
    - 完整的生命周期管理

2. **HTTP API 层（Axum）**
    - `POST /write` - Raft 一致性写入（client_write）
    - `GET /read` - 本地读取（非 Raft）
    - `GET /membership` - 查看集群成员
    - `POST /change_membership` - 动态变更成员
    - `GET /metrics` - Prometheus 指标导出
    - `GET /health` - 健康检查

3. **完整的 Prometheus 集成**
    - OpenRaft 内置指标（term, index, etc.）
    - 自定义指标：merge 时长、snapshot 链长、delta 大小
    - SurrealKV 内部指标：写入延迟、压缩时间
    - HTTP API 指标：请求延迟、错误率

4. **Graceful Shutdown**
    - 信号处理（SIGTERM, Ctrl+C）
    - 等待中途请求完成
    - 持久化未应用的状态
    - 资源清理（连接、文件）

5. **配置管理**
    - 环境变量支持：NODE_ID, CLUSTER_NODES, LISTEN_ADDR, etc.
    - 配置文件支持（TOML/YAML）
    - 默认配置预设

6. **日志与追踪**
    - tracing 集成（结构化日志）
    - 分级日志输出（DEBUG/INFO/WARN/ERROR）
    - OpenTelemetry 预留（扩展点）

7. **Docker 支持**
    - Dockerfile.multi - 多阶段生产构建
    - docker-compose.yml - 三节点本地集群
    - entrypoint.sh - 启动脚本

8. **三节点集群启动验证**
    - examples/3nodes-local.sh - 一键启动脚本
    - 自动生成配置文件
    - 健康检查与日志验证

### 文件规划

- `src/main.rs` - NEW：应用入口与启动流程
    - 主函数、信号处理、graceful shutdown
- `src/app.rs` - NEW：RaftNode 业务逻辑与集成
    - RaftNode 结构与生命周期
    - client_write 与 apply 流程
- `src/http.rs` - NEW：Axum HTTP API 服务器
    - 路由定义、处理器实现
    - 错误处理与响应格式
- `src/config.rs` - NEW：配置管理与环境变量解析
    - Config 结构、加载与验证
- `src/metrics.rs` - NEW/扩展：Prometheus 指标导出
    - 指标定义、采集、导出
    - 内置指标 + 自定义指标
- `src/lib.rs` - 修改：导出所有模块
- `docker/Dockerfile.multi` - NEW：多阶段 Docker 构建
- `docker/docker-compose.yml` - NEW：三节点集群编排
- `docker/entrypoint.sh` - NEW：启动脚本
- `examples/3nodes-local.sh` - NEW/修改：本地启动脚本
- `tests/integration_test.rs` - NEW：完整集成测试

### 关键 API

```rust
/// RaftNode - 整合应用
pub struct RaftNode {
    pub raft: openraft::Raft<RaftTypeConfig>,
    pub storage: Arc<SurrealStorage>,
    pub network: Arc<RaftNetworkFactory>,
    pub config: Arc<Config>,
}

impl RaftNode {
    /// 初始化三节点集群
    pub async fn init_cluster(nodes: Vec<(NodeId, String)>) -> Result<Self>;

    /// 客户端写入（Raft 一致性）
    pub async fn client_write(&self, req: KVRequest) -> Result<KVResponse>;

    /// 本地读取（非 Raft）
    pub async fn read(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// 查询成员列表
    pub async fn membership(&self) -> Result<Vec<(NodeId, BasicNode)>>;

    /// 动态变更成员（添加学习者→提交变更）
    pub async fn change_membership(&self, members: Vec<(NodeId, BasicNode)>) -> Result<()>;
}

/// HTTP 服务器
pub async fn start_http_server(
    addr: &str,
    node: Arc<RaftNode>,
) -> Result<()>;
```

### HTTP API 示例

```bash
# 写入
curl -X POST http://localhost:8080/write \
  -H "Content-Type: application/json" \
  -d '{"Set":{"key":"mykey","value":"aGVsbG8="}}'
# Response: {"Ok": true}

# 读取
curl http://localhost:8080/read?key=mykey
# Response: {"Value": "aGVsbG8="}

# 集群成员
curl http://localhost:8080/membership
# Response: [{"1": "localhost:50051"}, {"2": "localhost:50052"}, {"3": "localhost:50053"}]

# 指标导出
curl http://localhost:8080/metrics
# Prometheus 格式输出
```

### 三节点启动流程

```bash
./examples/3nodes-local.sh
# 启动 3 个节点（各自独立 SurrealKV 引擎）
# Node 1: localhost:50051 (gRPC), :8080 (HTTP)
# Node 2: localhost:50052 (gRPC), :8081 (HTTP)
# Node 3: localhost:50053 (gRPC), :8082 (HTTP)
# 自动初始化 Node 1 为 leader
# 验证日志复制与快照流程
```

### 验证点

- [ ] HTTP API 端点全覆盖测试
- [ ] 三节点本地集群启动脚本通过
- [ ] Leader 选举、日志复制、快照完整走通
- [ ] 压力测试：1000 QPS、快照过程中一致性
- [ ] 单节点故障恢复（kill node 2 & restart）
- [ ] 网络分割恢复（kill node 1，node 2/3 重新选举）
- [ ] Docker 镜像构建与运行
- [ ] docker-compose 三节点启动成功
- [ ] Prometheus 指标完整性（15+ 指标）
- [ ] Graceful shutdown 验证

### 交付物

- 完整的应用启动框架
- HTTP API 服务器与路由（5+ endpoints）
- 配置管理系统
- Docker 单节点与多节点编排
- 集成测试套件（20+ test cases）
- 生产部署文档与监控配置
- 性能基准报告（吞吐量、延迟、资源占用）

### 性能目标

- 单节点写 QPS: >5k（内存状态机）
- 集群 QPS（3 节点）: >1k（Raft consensus）
- 快照传输：<2s（100MB 快照）
- P99 延迟：<50ms（本地网络）
- 内存占用：<500MB（单节点）

### 预计时间表

- 周 1: RaftNode 架构与 HTTP API (25h)
- 周 2: 配置、日志、Docker (25h)
- 周 3: 集成测试与性能调优 (30h)

---

## 总体工作量估算

| 阶段      | 开发 (h)  | 测试 (h)  | 文档 (h) | 小计 (h)  | 状态        |
|---------|---------|---------|--------|---------|-----------|
| Phase 0 | 50      | 20      | 10     | **80**  | ✅ 完成      |
| Phase 1 | 40      | 20      | 10     | **70**  | 🔄 就绪     |
| Phase 2 | 40      | 20      | 10     | **70**  | 📋 设计     |
| Phase 3 | 30      | 15      | 5      | **50**  | 🔄 v2 进行中 |
| Phase 4 | 60      | 20      | 10     | **90**  | 📋 设计     |
| Phase 5 | 50      | 20      | 10     | **80**  | 📋 设计     |
| **合计**  | **270** | **115** | **55** | **440** | -         |

---

## 依赖关系与进度管理

```
Phase 0 (基础框架) ✅ 完成
    ↓ (必要前置)
Phase 1 (网络层) 🔄 就绪
    ↓ (网络层必需)
Phase 2 (Checkpoint) 📋 设计
    ↓ (快照必需)
Phase 3 (Delta) 🔄 v2 实现中
    ↓ (Delta + Checkpoint)
Phase 4 (Hybrid 合并) 📋 设计
    ↓ (所有存储功能)
Phase 5 (生产优化) 📋 设计

并行可能：Phase 1、2、3 某些部分可穿插进行（但需在 Phase 0 完全就位后）
```

---

## 风险评估与缓解措施

| 风险                   | 影响           | 概率 | 缓解措施                      |
|----------------------|--------------|----|---------------------------|
| **SurrealKV API 变更** | Phase 2-5 失效 | 中  | 定期同步更新，维护兼容适配层            |
| **Checkpoint 一致性**   | 数据损坏         | 低  | Phase 2 详细一致性验证 & 恢复测试    |
| **后台合并冲突**           | 快照状态混乱       | 低  | Phase 4 细粒度状态锁 & 原子操作     |
| **网络大包丢失**           | 快照传输中断       | 中  | Phase 1.5 分块传输 & 断点续传     |
| **性能不符目标**           | 生产部署困难       | 中  | Phase 5 压力测试 & 针对性优化      |
| **开发进度延期**           | 项目延期         | 中  | 严格的 sprint 规划 & 每周 review |

---

## 成功交付标准

### Phase 0 ✅

- ✅ cargo build 通过，无编译错误
- ✅ 30+ 单元测试通过
- ✅ 代码覆盖率 >75%

### Phase 1

- [ ] 两节点 gRPC 通信集成测试通过
- [ ] RPC 延迟基准 <100ms
- [ ] HTTP/2 连接复用验证

### Phase 2

- [ ] Checkpoint 创建→恢复流程通过
- [ ] 数据完整性验证（对比 checksum）
- [ ] 恢复时间 <2s

### Phase 3

- [x] Delta 构建与重放一致性验证
- [x] Delta 大小 <5MB
- [x] 版本兼容性测试通过

### Phase 4

- [ ] 后台合并不阻塞主线程（压力测试 1000 QPS）
- [ ] 原子状态切换正确性验证
- [ ] Prometheus 指标导出完整

### Phase 5

- [ ] 三节点本地集群启动成功
- [ ] API 调用返回 200 OK
- [ ] Prometheus 指标采集正常
- [ ] Docker 镜像构建与运行通过
- [ ] 集成测试 20+ cases 全通过
- [ ] Leader 选举、日志复制、快照完整走通
- [ ] 单节点故障恢复测试通过

---

## 技术栈总结

| 组件           | 库/框架        | 版本              | 用途               |
|--------------|-------------|-----------------|------------------|
| **共识**       | OpenRaft    | 0.10.0-alpha.15 | Raft 实现          |
| **存储**       | SurrealKV   | 0.20.2          | LSM 存储引擎         |
| **网络**       | Tonic       | 0.14.5          | gRPC 框架          |
| **Protobuf** | prost       | 0.14            | Protocol buffers |
| **异步运行时**    | Tokio       | 1.50            | 异步执行             |
| **序列化**      | postcard    | 1.0             | 二进制格式            |
| **压缩**       | zstd        | 0.13.2          | Zstandard 压缩     |
| **HTTP**     | Axum        | 0.8.8           | Web 框架           |
| **监控**       | Prometheus  | 0.18.1          | 指标导出             |
| **日志**       | tracing     | 0.1.44          | 结构化日志            |
| **测试**       | tokio::test | -               | 异步测试             |

---

## 文档与资源

### 官方文档

- [OpenRaft Docs](https://docs.rs/openraft/latest/openraft/)
- [SurrealKV Docs](https://docs.rs/surrealkv/latest/surrealkv/index.html)
- [Tonic Docs](https://docs.rs/tonic/latest/tonic/)

### 代码结构

- `IMPLEMENTATION_STATUS.md` - Phase 0 完成状态
- `ROADMAP.md` - 本文档（总体规划）
- `src/` - 所有模块源代码
- `proto/raft.proto` - gRPC 服务定义
- `examples/3nodes-local.sh` - 启动脚本
- `docker/` - Docker 配置

---

## 下一步行动

### 立即启动 Phase 1

1. 开发 src/network/client.rs（gRPC 客户端）
2. 开发 src/network/server.rs（RaftService 实现）
3. 完善 RaftNetworkFactory 与 RaftNetworkV2 trait
4. 实现连接池与超时管理
5. 编写两节点集成测试

**预计开始**: 2026-03-07  
**预计完成**: 2026-03-20
