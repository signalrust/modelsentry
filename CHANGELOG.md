# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog and this project adheres to Semantic Versioning.

## [Unreleased]

No unreleased changes.

## [0.1.0-init.0] - 2026-03-06

### Added

- Rust workspace scaffolding with five crates (`common`, `core`, `store`, `daemon`, `cli`)
- SvelteKit frontend scaffold under `web/`
- Shared workspace dependencies, lint policy, and toolchain setup
- Common crate Phase 1: newtype IDs, `ApiKey` with redacted serde, error hierarchy, domain models, TOML config with validation — 32 tests
- Enterprise repository governance (`LICENSE`, `SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, Dependabot)
- CI pipeline: Rust (check/fmt/clippy/test), security (cargo audit), frontend (check/build/audit)
- CodeQL static analysis and dependency review workflows
- Initial project architecture and phased execution plan docs
