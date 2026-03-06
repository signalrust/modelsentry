.PHONY: check fmt fmt-check lint test audit web-install web-check ci

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

ci: fmt-check lint test audit web-check
