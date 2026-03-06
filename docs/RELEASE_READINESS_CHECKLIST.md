# Release Readiness Checklist

Use this checklist before creating any public release tag.

## Engineering Quality

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd web && npm run check && npm run build`

## Security

- [ ] `cargo audit` returns no unfixed vulnerabilities
- [ ] `npm audit --audit-level=high` returns no unfixed high/critical vulnerabilities
- [ ] No secrets in code, docs, fixtures, or CI logs
- [ ] `SECURITY.md` reporting path is verified

## Documentation

- [ ] `README.md` status and test counts are up to date
- [ ] `CHANGELOG.md` updated for release
- [ ] Architecture/plan docs updated for any behavior changes

## Governance

- [ ] `LICENSE` is present and accurate
- [ ] `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and issue templates are present
- [ ] `CODEOWNERS` reflects current maintainers

## Release Metadata

- [ ] Version is set appropriately
- [ ] Tag and release notes are prepared
- [ ] Rollback strategy is documented
