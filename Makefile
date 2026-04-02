.PHONY: check fmt fmt-check lint test audit web-install web-check web-audit ci setup

setup:
	git config core.hooksPath .githooks
	@echo "Git hooks installed. Pre-commit and pre-push checks are now active."

check:
	cargo check --workspace --all-targets

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

audit:
	cargo audit

web-install:
	cd web && npm ci

web-check:
	cd web && npm run check && npm run build

web-audit:
	cd web && npm audit --audit-level=high

ci: fmt-check lint test audit web-check web-audit
