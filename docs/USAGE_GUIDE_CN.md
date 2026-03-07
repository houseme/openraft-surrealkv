# OpenRaft-SurrealKV 使用指南

本文档基于当前代码实现（`v0.0.2`），说明如何构建、配置、运行、观测与排障 `openraft-surrealkv`。

## 1. 功能概览

`openraft-surrealkv` 组合了以下能力：

- 使用 OpenRaft 运行时（`openraft 0.10.0-alpha.15`）实现一致性
- 使用 SurrealKV 作为本地存储引擎
- 使用 gRPC 进行 Raft 节点间通信
- 提供 HTTP API 进行 KV 读写与健康探测
- 提供 Prometheus 兼容指标输出
- 提供全量 + 增量快照，以及合并/恢复机制

核心模块：

- `src/main.rs`：进程启动、服务编排、优雅退出
- `src/config.rs`：配置模型、CLI/env 覆盖、严格校验
- `src/app.rs`：`RaftNode` 封装与写入路径
- `src/api/`：HTTP 路由（`/kv`、`/health`、`/ready`、`/status`、`/metrics`）
- `src/metrics/`：指标名称与记录逻辑
- `src/merge/error_codes.rs`：稳定 merge 错误码

## 2. 前置要求

- Rust 工具链（兼容 `rust-version = 1.93`）
- macOS/Linux shell（示例使用 `bash`/`zsh`）
- `curl`（用于 API 调试）
- 可选：`jq`（格式化 JSON 输出）

## 3. 构建与测试

在仓库根目录执行：

```bash
cargo check
cargo test
```

特性门控的集群集成测试：

```bash
cargo test --features integration-cluster --test cluster_observability
```

## 4. 配置模型

主配置示例文件：`config.toml.example`

配置分区：

- `[node]`：节点身份、Raft gRPC 监听地址、数据目录
- `[http]`：HTTP API 开关与端口
- `[raft]`：心跳/选举参数、单次载荷条目上限
- `[cluster]`：bootstrap 与 peers 拓扑
- `[snapshot]`：checkpoint 与 delta 合并阈值
- `[storage]`：存储参数
- `[logging]`：日志级别与格式（`text`/`json`）
- `[metrics]`：指标配置字段

### 4.1 覆盖优先级

`Config::load()` 的优先级顺序：

1. 默认值
2. 配置文件（`--config`）
3. CLI/env 覆盖

支持的覆盖参数/环境变量：

- `--node-id` / `NODE_ID`
- `--listen-addr` / `LISTEN_ADDR`
- `--data-dir` / `DATA_DIR`
- `--http-port` / `HTTP_PORT`
- `--log-level` / `LOG_LEVEL`

### 4.2 严格校验规则

`Config::validate()` 会校验：

- `node_id > 0`
- `listen_addr` 非空
- `heartbeat_interval_ms < election_timeout_ms`
- 若启用 HTTP，`http.port != 0`
- peers 不能包含自身
- peers 的 node_id 与 addr 都必须唯一
- `expected_voters`（若设置）必须 `> 0` 且等于 `1 + peers.len()`

## 5. 单节点运行

### 5.1 最小配置

创建 `config.single.toml`：

```toml
[node]
node_id = 1
listen_addr = "127.0.0.1:50051"
data_dir = "./data/node_1"

[http]
enabled = true
port = 8080

[raft]
heartbeat_interval_ms = 500
election_timeout_ms = 3000
max_payload_entries = 300

[cluster]
bootstrap = false
peers = []

[snapshot]
checkpoint_interval_secs = 3600
max_delta_chain = 5
max_delta_bytes_mb = 300

[storage]
enable_compression = true
flush_interval_ms = 1000

[logging]
level = "info"
format = "text"

[metrics]
enabled = true
listen_addr = "0.0.0.0:9090"
```

### 5.2 启动

```bash
cargo run -- --config config.single.toml
```

## 6. 本地三节点运行

为每个节点准备一份配置。示例拓扑：

- Node 1: Raft `127.0.0.1:50051`, HTTP `8080`
- Node 2: Raft `127.0.0.1:50052`, HTTP `8081`
- Node 3: Raft `127.0.0.1:50053`, HTTP `8082`

关键规则：

- 仅一个节点设置 `cluster.bootstrap = true`（通常 node 1）
- 所有节点保持 `cluster.expected_voters = 3`
- `cluster.peers` 只填写其他节点

### 6.1 Node 1 配置示例（`config.node1.toml`）

```toml
[node]
node_id = 1
listen_addr = "127.0.0.1:50051"
data_dir = "./data/node_1"

[http]
enabled = true
port = 8080

[raft]
heartbeat_interval_ms = 500
election_timeout_ms = 3000
max_payload_entries = 300

[cluster]
bootstrap = true
expected_voters = 3

[[cluster.peers]]
node_id = 2
addr = "127.0.0.1:50052"

[[cluster.peers]]
node_id = 3
addr = "127.0.0.1:50053"

[snapshot]
checkpoint_interval_secs = 3600
max_delta_chain = 5
max_delta_bytes_mb = 300

[storage]
enable_compression = true
flush_interval_ms = 1000

[logging]
level = "info"
format = "text"

[metrics]
enabled = true
listen_addr = "0.0.0.0:9090"
```

Node 2/3 仅需替换 `node_id`、`listen_addr`、`data_dir`、`http.port` 与 peers 对应配置。

### 6.2 三终端启动

```bash
cargo run -- --config config.node1.toml
cargo run -- --config config.node2.toml
cargo run -- --config config.node3.toml
```

## 7. HTTP API

路由定义在 `src/api/server.rs`，处理逻辑在 `src/api/handlers.rs`。

### 7.1 端点列表

- `GET /health`：存活探针
- `GET /ready`：就绪探针（进程 + 存储）
- `GET /status`：节点角色/任期/已应用索引
- `GET /metrics`：Prometheus 文本格式指标
- `POST /kv/:key`：将请求体原始字节写入 value
- `GET /kv/:key`：读取 value（Base64 编码）
- `DELETE /kv/:key`：删除 key

### 7.2 API 示例

写入：

```bash
echo -n "hello" | curl -sS -X POST http://127.0.0.1:8080/kv/demo -d @-
```

读取：

```bash
curl -sS http://127.0.0.1:8080/kv/demo
```

返回结构：

```json
{
  "value": "aGVsbG8="
}
```

Base64 解码（`macOS`）：

```bash
echo 'aGVsbG8=' | base64 -D
```

删除：

```bash
curl -sS -X DELETE http://127.0.0.1:8080/kv/demo
```

状态与探针：

```bash
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/ready
curl -sS http://127.0.0.1:8080/status
```

## 8. 可观测性

### 8.1 指标端点

在各节点 HTTP 端口抓取 `GET /metrics`：

```bash
curl -sS http://127.0.0.1:8080/metrics
```

### 8.2 稳定 Merge 错误码

定义于 `src/merge/error_codes.rs`：

- `MERGE_BASELINE_MISSING`
- `MERGE_BASELINE_PATH_REQUIRED`
- `MERGE_BASELINE_PATH_MISSING`
- `MERGE_INJECTED_FAILURE`
- `MERGE_UNKNOWN`

### 8.3 Merge/Snapshot 指标名称

来自 `src/metrics/mod.rs`：

- `raft_snapshot_merge_duration_ms`
- `raft_snapshot_merge_success_total`
- `raft_snapshot_merge_failed_total`
- `raft_snapshot_checkpoint_size_bytes`
- `raft_snapshot_delta_chain_length`
- `raft_snapshot_delta_cumulative_mb`

严格快照错误计数器：

- `snapshot_strict_error_decode_payload_total`
- `snapshot_strict_error_validate_base_total`
- `snapshot_strict_error_apply_delta_total`
- `snapshot_strict_error_reject_payload_total`

## 9. 快照与合并运行行为

基于 `src/snapshot.rs` 与启动流程：

- 全量/增量快照使用 envelope 编码
- 增量快照选择受 baseline、条目阈值、payload 大小与时间窗口约束
- 快照元数据包含 `last_log_id`、`snapshot_id`、membership baseline
- merge policy 在启动时由 `[snapshot]` 配置构建
- merge 恢复检查在服务对外前执行

## 10. 数据与运行路径

进程常见运行路径：

- SurrealKV 树：`<data_dir>/kv`
- Checkpoint：`target/checkpoints`
- Delta 临时文件：`target/tmp/deltas`
- 全量快照输出：`target/snapshots`
- 恢复输出：`target/restored`

## 11. 运维说明

- 启动前会执行 preflight：数据目录可写、端口可绑定。
- 若 Raft 初始化失败，进程可回退 standalone 模式（走本地存储写路径）。
- `Ctrl+C` 时会触发服务停止与 Raft shutdown。

## 12. 故障排查

### 配置校验失败

常见报错（来自 `Config::validate()`）：

- `heartbeat_interval_ms must be less than election_timeout_ms`
- `cluster.peers contains duplicate node_id: ...`
- `cluster.peers must not include current node_id`
- `cluster.expected_voters mismatch: expected=..., actual=...`

### 就绪检查失败

若 `/ready` 返回 `ready=false`，优先检查 `details.last_error`，并确认数据目录权限和磁盘状态。

### 集群无法选主

- 检查各节点 Raft `listen_addr` 连通性
- 确认 peer 配置对称、节点 ID 正确
- 首次初始化时确认仅一个 bootstrap 节点

## 13. v0.0.2 发布/升级说明

- 当前包版本：`0.0.2`（见 `Cargo.toml`）
- 启动日志会打印 `openraft_surrealkv::VERSION`
- 详细变更请参考 `CHANGELOG.md`

## 14. 快速命令清单

```bash
# 构建
cargo check

# 单节点运行
cargo run -- --config config.single.toml

# 运行测试
cargo test

# 集群集成测试（feature-gated）
cargo test --features integration-cluster --test cluster_observability

# 健康与状态
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/status
```

