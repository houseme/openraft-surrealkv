# AGENT Operating Notes

This repository contains an iterative, developer-led implementation of a Raft-backed
key-value store built on top of OpenRaft and SurrealKV.

## Purpose

Provide a short summary for automated agents (test runners / code generators) about
repository conventions and important modules.

## Conventions

- `src/` organizes core modules: `storage`, `merge`, `snapshot`, `state`, `api`, `network`.
- Tests:
    - Unit tests live inside `src/*` with `#[cfg(test)]`.
    - Integration tests live in `tests/` with harness code under `tests/common/`.
- Feature gating:
    - `integration-cluster` enables heavy multi-node tests.

## Important files for agents

- `src/config.rs` - Configuration struct and validation logic (including CLI args).
- `src/merge/error_codes.rs` - Canonical merge error constants for metrics and logs.
- `src/merge/executor.rs` - Merge orchestration and retry logic.
- `src/storage.rs` - SurrealStorage core and OpenRaft trait wiring.
- `README.md` / `README_CN.md` - High-level run and SRE guidance.

## Testing recommendations for agents

- Prefer unit tests first (`cargo test`).
- Run targeted integration tests with feature gate:
  `cargo test --features integration-cluster --test merge_e2e`

## Pre-commit checks

Before submitting changes, ensure the following commands pass to maintain code quality:

1. **Format code**:
   ```bash
   cargo fmt --all
   ```

2. **Lint and fix (Clippy)**:
   ```bash
   cargo clippy --fix --all-targets --all-features --allow-dirty -- -D warnings
   ```

3. **Apply automatic fixes**:
   ```bash
   cargo fix --lib -p openraft-surrealkv --allow-dirty && cargo fmt --all
   ```

4. **Verify compilation**:
   ```bash
   cargo check --all-targets --all-features -p openraft-surrealkv
   ```

5. **Run unit tests**:
   ```bash
   cargo test
   ```

6. **Final formatting**:
   ```bash
   cargo fmt --all
   ```

## Editing and commit policy

- Keep changes focused and small; include unit tests for behavior changes.
- When modifying error codes or metrics, adjust both `src/merge/error_codes.rs` and
  `README*.md` accordingly.
- Ensure `CliArgs` in `src/config.rs` is updated if new command-line arguments are added.

## Contact

- Maintainers: see `Cargo.toml` authors field.
