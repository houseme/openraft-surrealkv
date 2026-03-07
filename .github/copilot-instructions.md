# GitHub Copilot / Automated Assistant Instructions

This document provides guidance for code-completion agents and automated assistants
that interact with this repository.

## Primary goals

- Maintain clear, well-documented code with stable public interfaces.
- Keep tests green: unit tests are required for behavior changes.
- Preserve stable metrics/labels and error-code strings used by SRE.

## Style and expectations

- Prefer small, focused edits. Each PR should contain a brief description and tests.
- When translating comments, keep both human-readable context and technical details.
- Avoid changing public APIs without updating all call-sites and adding migration tests.
- Follow Rust idioms and `clippy` suggestions where applicable.

## Files of interest for automation

- `src/config.rs` - Configuration loading, validation, and CLI argument parsing.
- `src/merge/error_codes.rs` - Source of truth for merge-related error codes (metrics label).
- `src/metrics/mod.rs` - Metrics naming and label usage.
- `README.md` / `README_CN.md` - Runbook and SRE guidance; update when labels change.

## Testing and CI

Before proposing a commit, ensure the following checks pass:

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

Heavy integration tests are feature-gated (`integration-cluster`) to reduce CI time.

## Security

- Never commit secrets. Use environment variables for keys.

## If in doubt

- Open a small PR describing the intended change and request review from repository
  maintainers (authors listed in Cargo.toml).
