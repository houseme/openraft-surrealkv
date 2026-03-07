# OpenRaft-SurrealKV

基于 `OpenRaft 0.10.0-alpha.15` 与 `SurrealKV` 的分布式 KV 项目。

## 当前状态

- 已接入真实 `openraft::Raft` 运行时。
- 已实现 OpenRaft 存储 trait：
    - `RaftLogStorage`
    - `RaftLogReader`
    - `RaftStateMachine`
    - `RaftSnapshotBuilder`
- 支持配置驱动的多节点 bootstrap。
- 三节点选主与复制集成测试已具备，并通过 feature 开关控制执行，提升 CI 稳定性。

## 文档

- 中文使用指南：`docs/USAGE_GUIDE_CN.md`
- English Usage Guide: `docs/USAGE_GUIDE.md`

## 关键目录

```text
src/
  app.rs                       # RaftNode 封装（启动/写入/读取/关闭）
  config.rs                    # 配置模型与严格校验
  main.rs                      # 进程入口
  storage.rs                   # SurrealStorage 核心
  storage_raft_impl.rs         # OpenRaft trait 实现
  network/                     # gRPC 客户端/服务端

tests/
  common/cluster_harness.rs    # 三节点集成测试公共启动/关闭工具
  cluster_observability.rs     # 选主观测 + 复制可见测试
```

## 集群配置

参考 `config.toml.example`。

`[cluster]` 关键字段：

- `bootstrap`：当前节点是否执行 `initialize`
- `expected_voters`：期望 voter 数量（`self + peers`）
- `peers`：对端节点列表（`node_id`、`addr`）

`src/config.rs` 中的严格校验规则：

- `peers` 不允许重复 `node_id`
- `peers` 不允许重复 `addr`
- `peers` 中不能包含当前节点
- `peers.addr` 不能等于本地 `listen_addr`
- `expected_voters`（若设置）必须 `> 0`
- `expected_voters` 必须等于 `1 + peers.len()`（严格执行，与 `bootstrap` 无关）

## 运行方式

单节点：

```bash
cargo run -- --config config.toml.example
```

多节点：

1. 为每个节点准备独立配置。
2. 仅一个节点设置 `cluster.bootstrap = true`。
3. 所有节点的 `cluster.expected_voters` 保持一致。
4. 保证 peer 地址可达。

## 测试

常规测试：

```bash
cargo test
```

开启 feature 后运行三节点集成测试：

```bash
cargo test --features integration-cluster --test cluster_observability
```

运行单个集成用例：

```bash
cargo test --features integration-cluster --test cluster_observability test_three_node_election_observability
cargo test --features integration-cluster --test cluster_observability test_three_node_replication_visibility
```

## Merge 错误码与指标标签说明（SRE）

合并链路统一使用 `src/merge/error_codes.rs` 中的稳定 `MERGE_*` 错误码。

| 错误码                            | 含义                              | 常见触发场景              |
|--------------------------------|---------------------------------|---------------------|
| `MERGE_BASELINE_MISSING`       | 快照状态中没有 baseline checkpoint 元数据 | merge 启动时基线尚未写入     |
| `MERGE_BASELINE_PATH_REQUIRED` | 严格模式要求显式 `checkpoint_path`      | 旧/不完整元数据未携带路径       |
| `MERGE_BASELINE_PATH_MISSING`  | `checkpoint_path` 存在但目录不存在      | checkpoint 文件被清理或损坏 |
| `MERGE_INJECTED_FAILURE`       | 测试注入故障                          | 单元/集成测试故障注入         |
| `MERGE_UNKNOWN`                | 无法解析出已知错误码时的兜底值                 | 非标准 merge 错误文本      |

`raft_snapshot_merge_failed_total` 与失败样本 `raft_snapshot_merge_duration_ms` 的关键标签：

- `trigger`：触发原因（`chain_length` / `delta_bytes` / `checkpoint_interval`）
- `error_code`：稳定错误码（见上表）
- `node_id`、`retries`、`result=failed`

建议告警/看板维度：

- `error_code="MERGE_BASELINE_PATH_MISSING"` 的突增
- 同一 `node_id` 的失败重试次数持续升高
- 某一 `trigger` 触发路径异常集中

## 许可证

`MIT OR Apache-2.0`
