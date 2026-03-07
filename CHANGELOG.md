# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.0.2] - 2026-03-07

### Added

- Added a standalone, code-aligned usage document: `docs/USAGE_GUIDE.md`.
- Added comprehensive operator guidance for single-node and three-node deployment, API usage, observability, and
  troubleshooting.

### Changed

- Bumped crate version to `0.0.2` in `Cargo.toml`.
- Refactored snapshot internals in `src/snapshot.rs` to reduce duplication with private helper methods for membership
  and metadata construction while keeping public APIs unchanged.
- Refactored default-initialization patterns to satisfy strict clippy `-D warnings` flows in snapshot/app/cleanup code
  paths.

### Fixed

- Removed field reassign-after-default warning patterns in:
    - `src/merge/cleanup.rs`
    - `src/snapshot.rs`
    - `src/app.rs`
- Removed unused assignment warning in `tests/common/cluster_harness.rs`.

## [0.0.1] - 2026-03-05

### Changed

- Initial baseline release notes placeholder.
