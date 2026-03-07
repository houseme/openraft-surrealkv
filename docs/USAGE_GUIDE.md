# OpenRaft-SurrealKV Usage Guide

This guide explains how to build, configure, run, observe, and troubleshoot `openraft-surrealkv` based on the current
codebase (`v0.0.2`).

## 1. What You Get

`openraft-surrealkv` combines:

- OpenRaft runtime (`openraft 0.10.0-alpha.15`) for consensus
- SurrealKV as the local storage engine
- gRPC transport for Raft internode traffic
- HTTP API for key-value operations and probes
- Prometheus-compatible metrics endpoint
- Full + delta snapshot pipeline and merge/recovery logic

Key modules:

- `src/main.rs`: process startup, service wiring, graceful shutdown
- `src/config.rs`: config model, CLI/env overrides, strict validation
- `src/app.rs`: `RaftNode` wrapper and write path behavior
- `src/api/`: HTTP routes (`/kv`, `/health`, `/ready`, `/status`, `/metrics`)
- `src/metrics/`: metric names and recorders
- `src/merge/error_codes.rs`: stable merge error-code strings

## 2. Prerequisites

- Rust toolchain compatible with `rust-version = 1.93`
- macOS/Linux shell (examples use `bash`/`zsh`)
- `curl` for API checks
- Optional: `jq` for pretty JSON output

## 3. Build and Test

From repository root:

```bash
cargo check
cargo test
```

Feature-gated cluster integration tests:

```bash
cargo test --features integration-cluster --test cluster_observability
```

## 4. Configuration Model

Primary config file: `config.toml.example`

Config sections:

- `[node]`: identity, Raft gRPC listen address, data directory
- `[http]`: HTTP API enable flag and listen port
- `[raft]`: heartbeat/election timing and payload size
- `[cluster]`: bootstrap and peer topology
- `[snapshot]`: checkpoint and delta merge thresholds
- `[storage]`: storage tuning flags
- `[logging]`: level + format (`text` or `json`)
- `[metrics]`: metrics configuration fields

### 4.1 Override Precedence

As implemented by `Config::load()`:

1. Defaults
2. Config file (`--config`)
3. CLI/env overrides

Supported override args/env:

- `--node-id` / `NODE_ID`
- `--listen-addr` / `LISTEN_ADDR`
- `--data-dir` / `DATA_DIR`
- `--http-port` / `HTTP_PORT`
- `--log-level` / `LOG_LEVEL`

### 4.2 Strict Validation Rules

`Config::validate()` enforces:

- `node_id > 0`
- non-empty `listen_addr`
- `heartbeat_interval_ms < election_timeout_ms`
- if HTTP enabled, `http.port != 0`
- cluster peers cannot include self
- cluster peer IDs and addresses must be unique
- `expected_voters` (if set) must be `> 0` and equal to `1 + peers.len()`

## 5. Run a Single Node

### 5.1 Minimal Config

Create `config.single.toml`:

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

### 5.2 Start

```bash
cargo run -- --config config.single.toml
```

## 6. Run a 3-Node Cluster (Local)

Prepare one config per node. Example topology:

- Node 1: Raft `127.0.0.1:50051`, HTTP `8080`
- Node 2: Raft `127.0.0.1:50052`, HTTP `8081`
- Node 3: Raft `127.0.0.1:50053`, HTTP `8082`

Important rules:

- Set `cluster.bootstrap = true` on exactly one node (typically node 1)
- Keep `cluster.expected_voters = 3` on all nodes
- `cluster.peers` must list only other nodes

### 6.1 Example: Node 1 (`config.node1.toml`)

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

Create matching node2/node3 configs by swapping `node_id`, `listen_addr`, `data_dir`, `http.port`, and peer list.

### 6.2 Start in 3 Terminals

```bash
cargo run -- --config config.node1.toml
cargo run -- --config config.node2.toml
cargo run -- --config config.node3.toml
```

## 7. HTTP API

Routes are defined in `src/api/server.rs` and handlers in `src/api/handlers.rs`.

### 7.1 Endpoints

- `GET /health`: liveness check
- `GET /ready`: readiness check (process + storage probe)
- `GET /status`: node role/term/applied index
- `GET /metrics`: Prometheus exposition text
- `POST /kv/:key`: write raw request body as value bytes
- `GET /kv/:key`: read value (Base64-encoded JSON field)
- `DELETE /kv/:key`: delete key

### 7.2 API Examples

Write a key:

```bash
echo -n "hello" | curl -sS -X POST http://127.0.0.1:8080/kv/demo -d @-
```

Read the key:

```bash
curl -sS http://127.0.0.1:8080/kv/demo
```

The response shape is:

```json
{
  "value": "aGVsbG8="
}
```

Decode Base64 value (`macOS`):

```bash
echo 'aGVsbG8=' | base64 -D
```

Delete:

```bash
curl -sS -X DELETE http://127.0.0.1:8080/kv/demo
```

Status and probes:

```bash
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/ready
curl -sS http://127.0.0.1:8080/status
```

## 8. Observability

### 8.1 Metrics Endpoint

Scrape `GET /metrics` on each node HTTP port.

```bash
curl -sS http://127.0.0.1:8080/metrics
```

### 8.2 Stable Merge Error Codes

Defined in `src/merge/error_codes.rs`:

- `MERGE_BASELINE_MISSING`
- `MERGE_BASELINE_PATH_REQUIRED`
- `MERGE_BASELINE_PATH_MISSING`
- `MERGE_INJECTED_FAILURE`
- `MERGE_UNKNOWN`

### 8.3 Merge/Snapshot Metric Names

From `src/metrics/mod.rs`:

- `raft_snapshot_merge_duration_ms`
- `raft_snapshot_merge_success_total`
- `raft_snapshot_merge_failed_total`
- `raft_snapshot_checkpoint_size_bytes`
- `raft_snapshot_delta_chain_length`
- `raft_snapshot_delta_cumulative_mb`

Strict snapshot error counters:

- `snapshot_strict_error_decode_payload_total`
- `snapshot_strict_error_validate_base_total`
- `snapshot_strict_error_apply_delta_total`
- `snapshot_strict_error_reject_payload_total`

## 9. Snapshot and Merge Runtime Behavior

Based on `src/snapshot.rs` and startup wiring:

- Full and delta snapshot payloads are envelope-encoded
- Delta snapshot selection considers baseline existence, entry threshold, payload size, and age
- Snapshot metadata includes `last_log_id`, `snapshot_id`, and membership baseline
- Merge policy is built from `[snapshot]` config at startup
- Merge recovery check is executed during startup before serving traffic

## 10. Data and Runtime Paths

Common runtime paths used by the process:

- SurrealKV tree: `<data_dir>/kv`
- Checkpoints: `target/checkpoints`
- Delta temp files: `target/tmp/deltas`
- Full snapshot archive output: `target/snapshots`
- Restore output: `target/restored`

## 11. Operational Notes

- On startup, preflight verifies writable data directory and bindable ports.
- If Raft runtime initialization fails, process can continue in standalone mode (storage-backed write path).
- During shutdown (`Ctrl+C`), services stop and Raft shutdown is requested.

## 12. Troubleshooting

### Config validation failures

Typical errors from `Config::validate()`:

- `heartbeat_interval_ms must be less than election_timeout_ms`
- `cluster.peers contains duplicate node_id: ...`
- `cluster.peers must not include current node_id`
- `cluster.expected_voters mismatch: expected=..., actual=...`

### Readiness false

If `/ready` returns `ready=false`, inspect `details.last_error` and verify storage path permissions and disk health.

### Cluster does not elect leader

- Verify all Raft `listen_addr` values are reachable
- Ensure peer maps are symmetric and IDs are correct
- Ensure exactly one bootstrap node when first initializing

## 13. Release/Upgrade Notes for v0.0.2

- Package version is `0.0.2` (`Cargo.toml`)
- Startup log prints runtime version via `openraft_surrealkv::VERSION`
- Refer to `CHANGELOG.md` for release deltas

## 14. Quick Command Reference

```bash
# Build
cargo check

# Run one node
cargo run -- --config config.single.toml

# Run tests
cargo test

# Integration cluster test (feature-gated)
cargo test --features integration-cluster --test cluster_observability

# Health + status
curl -sS http://127.0.0.1:8080/health
curl -sS http://127.0.0.1:8080/status
```

