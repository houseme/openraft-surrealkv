AGENT Operating Notes

This repository contains an iterative, developer-led implementation of a Raft-backed
key-value store built on top of OpenRaft and SurrealKV.

Purpose

- Provide a short summary for automated agents (test runners / code generators) about
  repository conventions and important modules.

Conventions

- `src/` organizes core modules: `storage`, `merge`, `snapshot`, `state`, `api`, `network`.
- Tests:
    - Unit tests live inside `src/*` with `#[cfg(test)]`.
    - Integration tests live in `tests/` with harness code under `tests/common/`.
- Feature gating:
    - `integration-cluster` enables heavy multi-node tests.

Important files for agents

- `src/merge/error_codes.rs` - canonical merge error constants for metrics and logs
- `src/merge/executor.rs` - merge orchestration and retry logic
- `src/storage.rs` - SurrealStorage core and OpenRaft trait wiring
- `README.md` / `README_CN.md` - high-level run and SRE guidance

Testing recommendations for agents

- Prefer unit tests first (`cargo test`)
- Run targeted integration tests with feature gate:
  `cargo test --features integration-cluster --test merge_e2e`

Editing and commit policy

- Keep changes focused and small; include unit tests for behavior changes.
- When modifying error codes or metrics, adjust both `src/merge/error_codes.rs` and
  `README*.md` accordingly.

Contact

- Maintainers: see `Cargo.toml` authors field.

