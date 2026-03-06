# Contributing

## Development Setup

1. Install Rust stable with `clippy` and `rustfmt`
2. Install Node.js 20+ and npm
3. Run:

```bash
cargo check --workspace --all-targets
cd web && npm ci && npm run check
```

## Pull Request Requirements

Before opening a PR, ensure all checks pass:

```bash
make fmt-check
make lint
make test
make audit
make web-check
```

## Coding Standards

- Keep library code panic-free and return typed errors
- Avoid exposing secrets in logs, debug output, or serialized payloads
- Add tests for every behavior change
- Keep changes scoped and documented

## Commit Message Format

Use conventional prefixes where practical:

- `feat:` new behavior
- `fix:` bug or regression fix
- `chore:` maintenance/tooling/docs
- `refactor:` non-functional code structure improvement

## Documentation

If behavior changes, update:

- `README.md` for developer-facing usage
- `docs/ARCHITECTURE.md` for system-level impact
- `docs/PROJECT_PLAN.md` when implementation milestones shift
