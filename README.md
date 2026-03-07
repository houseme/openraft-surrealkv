# OpenRaft-SurrealKV

A distributed key-value project built with `OpenRaft 0.10.0-alpha.15` and `SurrealKV`.

## Status

- Real `openraft::Raft` runtime is wired in.
- OpenRaft storage traits are implemented:
    - `RaftLogStorage`
    - `RaftLogReader`
    - `RaftStateMachine`
    - `RaftSnapshotBuilder`
- Config-driven multi-node bootstrap is available.
- 3-node election and replication integration tests are available behind a feature flag for CI stability.

## Key Layout

```text
src/
  app.rs                       # RaftNode wrapper (startup/write/read/shutdown)
  config.rs                    # Config model + strict validation
  main.rs                      # Process entrypoint
  storage.rs                   # SurrealStorage core
  storage_raft_impl.rs         # OpenRaft trait implementations
  network/                     # gRPC client/server

tests/
  common/cluster_harness.rs    # Shared 3-node integration harness
  cluster_observability.rs     # Election + replication visibility integration tests
```

## Cluster Configuration

See `config.toml.example`.

Important fields in `[cluster]`:

- `bootstrap`: whether this node performs `initialize`
- `expected_voters`: expected voter count (`self + peers`)
- `peers`: peer list (`node_id`, `addr`)

Strict validation rules in `src/config.rs`:

- duplicate peer `node_id` is rejected
- duplicate peer `addr` is rejected
- local node must not appear in `peers`
- peer `addr` must not equal local `listen_addr`
- `expected_voters` must be `> 0` (if set)
- `expected_voters` must equal `1 + peers.len()` (strict, regardless of `bootstrap`)

## Run

Single node:

```bash
cargo run -- --config config.toml.example
```

Multi-node:

1. Prepare one config file per node.
2. Set `cluster.bootstrap = true` on exactly one node.
3. Keep `cluster.expected_voters` consistent across all nodes.
4. Ensure all peer addresses are reachable.

## Test

Regular tests:

```bash
cargo test
```

Feature-gated 3-node integration tests:

```bash
cargo test --features integration-cluster --test cluster_observability
```

Run one integration case:

```bash
cargo test --features integration-cluster --test cluster_observability test_three_node_election_observability
cargo test --features integration-cluster --test cluster_observability test_three_node_replication_visibility
```

## Merge Error Codes and Metrics Labels (SRE)

The merge pipeline uses stable `MERGE_*` codes from `src/merge/error_codes.rs`.

| Error Code                     | Meaning                                                 | Typical Trigger                           |
|--------------------------------|---------------------------------------------------------|-------------------------------------------|
| `MERGE_BASELINE_MISSING`       | No baseline checkpoint metadata in snapshot state       | merge starts before baseline is persisted |
| `MERGE_BASELINE_PATH_REQUIRED` | Strict mode requires explicit `checkpoint_path`         | legacy/incomplete metadata without path   |
| `MERGE_BASELINE_PATH_MISSING`  | `checkpoint_path` is set but directory does not exist   | checkpoint files removed/corrupted        |
| `MERGE_INJECTED_FAILURE`       | Test-only injected backend failure                      | integration/unit fault injection          |
| `MERGE_UNKNOWN`                | Fallback code when parser cannot extract a known prefix | non-standard merge error format           |

`raft_snapshot_merge_failed_total` and failed `raft_snapshot_merge_duration_ms` samples carry:

- `trigger`: merge trigger reason (`chain_length`, `delta_bytes`, `checkpoint_interval`)
- `error_code`: stable merge failure code (from table above)
- `node_id`, `retries`, `result=failed`

Example query dimensions for dashboards/alerts:

- high rate of `error_code="MERGE_BASELINE_PATH_MISSING"`
- repeated failures on same `node_id` with increasing `retries`
- spike on a specific `trigger`

## License

`MIT OR Apache-2.0`
