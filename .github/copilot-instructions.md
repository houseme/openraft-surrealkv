# GitHub Copilot / Automated Assistant Instructions

This document provides guidance for code-completion agents and automated assistants
that interact with this repository.

Primary goals

- Maintain clear, well-documented code with stable public interfaces.
- Keep tests green: unit tests are required for behavior changes.
- Preserve stable metrics/labels and error-code strings used by SRE.

Style and expectations

- Prefer small, focused edits. Each PR should contain a brief description and tests.
- When translating comments, keep both human-readable context and technical details.
- Avoid changing public APIs without updating all call-sites and adding migration tests.

Files of interest for automation

- `src/merge/error_codes.rs` - Source of truth for merge-related error codes (metrics label)
- `src/metrics/mod.rs` - Metrics naming and label usage
- `README.md` / `README_CN.md` - Runbook and SRE guidance; update when labels change

Testing and CI

- Run `cargo check` and `cargo test` locally before proposing a commit.
- Heavy integration tests are feature-gated (`integration-cluster`) to reduce CI time.

Security

- Never commit secrets. Use environment variables for keys.

If in doubt

- Open a small PR describing the intended change and request review from repository
  maintainers (authors listed in Cargo.toml).

