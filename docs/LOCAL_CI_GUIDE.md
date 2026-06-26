# Local CI: Pre-Commit & Pre-Push Checks

This document explains the full local CI environment for ModelSentry — the tools, configuration, hooks, and workflow that prevent broken code from ever reaching GitHub.

---

## Table of Contents

1. [Overview](#overview)
2. [Toolchain Prerequisites](#toolchain-prerequisites)
3. [Rust Toolchain Configuration](#rust-toolchain-configuration)
4. [Cargo Tools Explained](#cargo-tools-explained)
5. [Frontend Tools Explained](#frontend-tools-explained)
6. [Git Hooks Architecture](#git-hooks-architecture)
7. [Makefile Targets](#makefile-targets)
8. [Setup Instructions](#setup-instructions)
9. [Workflow: What Happens When You Commit](#workflow-what-happens-when-you-commit)
10. [Workflow: What Happens When You Push](#workflow-what-happens-when-you-push)
11. [How Local Hooks Map to CI](#how-local-hooks-map-to-ci)
12. [Troubleshooting](#troubleshooting)

---

## Overview

ModelSentry uses a **two-layer defense** to catch problems before they reach CI:

| Layer | Trigger | Checks | Speed |
|-------|---------|--------|-------|
| **Pre-commit hook** | Every `git commit` | Formatting, linting, type checking | Fast (~10s) |
| **Pre-push hook** | Every `git push` | All of the above + tests + security audits | Full (~30s) |
| **GitHub Actions CI** | Every push/PR on GitHub | Identical checks on ubuntu-latest | Final safety net |

The goal: **if your local hooks pass, CI will pass.** No surprises.

---

## Toolchain Prerequisites

### Rust (via rustup)

| Component | Purpose | Install |
|-----------|---------|---------|
| `rustc` | Rust compiler | `rustup toolchain install stable` |
| `cargo` | Build system & package manager | Bundled with rustc |
| `clippy` | Linter (catches bugs, anti-patterns) | Declared in `rust-toolchain.toml` |
| `rustfmt` | Code formatter (enforces style) | Declared in `rust-toolchain.toml` |
| `cargo-audit` | Security vulnerability scanner | `cargo install cargo-audit` |

### GNU Toolchain (Windows)

On Windows, Rust needs a C linker. We use the **GNU toolchain** (MinGW) instead of MSVC:

| Component | Purpose | Install |
|-----------|---------|---------|
| `gcc.exe` | C compiler & linker | MSYS2: `pacman -S mingw-w64-x86_64-gcc` |
| MinGW runtime | Windows API bindings for GNU | Bundled with the GNU Rust target |

The GNU toolchain is set as a directory override:

```sh
rustup override set stable-x86_64-pc-windows-gnu
```

This means: inside the `modelsentry/` directory, Rust always uses `x86_64-pc-windows-gnu` as the compilation target, which bundles its own linker. No Visual Studio or Windows SDK required.

> **Note:** CI runs on `ubuntu-latest` where the default `x86_64-unknown-linux-gnu` target works out of the box. The local GNU override is a Windows-only concern and does not affect CI.

### Node.js (for frontend)

| Component | Purpose | Install |
|-----------|---------|---------|
| `node` v22+ | JavaScript runtime | `winget install OpenJS.NodeJS.LTS` |
| `npm` | Package manager | Bundled with Node.js |

---

## Rust Toolchain Configuration

### `rust-toolchain.toml`

```toml
[toolchain]
channel = "stable"
components = ["clippy", "rustfmt"]
```

This file lives at the project root. When any `cargo` or `rustc` command runs inside this directory, `rustup` reads it and automatically:

1. **Installs** the `stable` toolchain if not present
2. **Adds** the `clippy` and `rustfmt` components
3. **Uses** this toolchain for all builds

This ensures every developer and CI runner uses the same Rust version and has the same tools.

### How `rustup override` interacts with `rust-toolchain.toml`

Priority order (highest first):
1. `rustup override` for the directory → `stable-x86_64-pc-windows-gnu`
2. `rust-toolchain.toml` in the project → `stable` channel with components
3. `rustup default` → global default

On your Windows machine, the override wins, selecting the GNU host. The `rust-toolchain.toml` still ensures `clippy` and `rustfmt` are present. On CI (Linux), there is no override, so `rust-toolchain.toml` controls everything.

---

## Cargo Tools Explained

### `cargo fmt` — Code Formatter

**What it does:** Enforces a consistent code style across all Rust files using `rustfmt`.

```sh
# Check formatting (fails if anything is wrong)
cargo fmt --all -- --check

# Auto-fix formatting
cargo fmt --all
```

**Flags:**
- `--all` — Format all workspace crates (not just the root)
- `--check` — Don't modify files, just report violations (used in hooks/CI)

**Configuration:** Uses the default `rustfmt` rules. Can be customized via a `rustfmt.toml` file at the project root.

**When it runs:** Pre-commit hook, pre-push hook, CI (`rust` job).

---

### `cargo clippy` — Linter

**What it does:** Runs hundreds of lint rules that catch common mistakes, performance issues, and non-idiomatic code. It's like a code reviewer that never sleeps.

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

**Flags:**
- `--workspace` — Lint all crates in the workspace
- `--all-targets` — Lint library, binary, test, bench, and example targets
- `-D warnings` — Treat all warnings as errors (deny warnings)

**Examples of what clippy catches:**
- Unused variables or imports
- Using `.unwrap()` where error handling is needed
- Inefficient patterns like `vec.len() == 0` instead of `vec.is_empty()`
- Potential panics in production code
- Redundant clones

**When it runs:** Pre-commit hook, pre-push hook, CI (`rust` job).

---

### `cargo test` — Test Runner

**What it does:** Compiles and runs all unit tests, integration tests, and doc tests across the entire workspace.

```sh
cargo test --workspace
```

**Flags:**
- `--workspace` — Test all crates, not just the root package

**Test locations in this project:**
- `crates/*/src/**` — Unit tests (`#[cfg(test)]` modules)
- `crates/core/tests/` — Integration tests
- Doc comments with code blocks — Doc tests

**When it runs:** Pre-push hook, CI (`rust` job). **Not** pre-commit (too slow for every commit).

---

### `cargo audit` — Security Scanner

**What it does:** Checks all dependencies in `Cargo.lock` against the [RustSec Advisory Database](https://rustsec.org/advisories/) for known vulnerabilities.

```sh
cargo audit
```

**Install:**
```sh
cargo install cargo-audit
```

**How it works:**
1. Fetches the advisory database from GitHub
2. Reads your `Cargo.lock`
3. Cross-references every dependency version against known CVEs
4. Exits with non-zero code if any vulnerability is found

**When it runs:** Pre-push hook, CI (`security` job).

**Fixing audit failures:**
```sh
# Update all dependencies to latest compatible versions
cargo update

# Check specific advisory
cargo audit --ignore RUSTSEC-2026-XXXX  # (temporary, not recommended)
```

---

### `cargo check` — Fast Compilation Check

**What it does:** Runs the compiler front-end without producing binaries. Much faster than `cargo build`.

```sh
cargo check --workspace --all-targets
```

Used in the Makefile `check` target for quick type-checking during development. Not directly used in hooks (clippy subsumes it).

---

## Frontend Tools Explained

### `svelte-check` — TypeScript & Svelte Type Checker

**What it does:** Runs the Svelte compiler and TypeScript checker on all `.svelte` and `.ts` files.

```sh
cd web && npm run check
# Which runs: svelte-kit sync && svelte-check --tsconfig ./tsconfig.json
```

- `svelte-kit sync` — Generates type definitions for SvelteKit routes
- `svelte-check` — Validates types, reports errors

**When it runs:** Pre-commit hook (only if `web/` files are staged), pre-push hook, CI (`frontend` job).

### `vite build` — Frontend Build

```sh
cd web && npm run build
```

Compiles the SvelteKit application for production. Catches any build-time errors that type checking alone might miss (e.g., missing imports at bundle time).

**When it runs:** Pre-push hook, CI (`frontend` job).

### `npm audit` — NPM Security Scanner

```sh
npm audit --audit-level=high
```

Checks `package-lock.json` against the npm advisory database. Only fails on **high** or **critical** severity vulnerabilities (ignores moderate/low).

**Fixing audit failures:**
```sh
cd web
npm audit fix          # Auto-fix compatible updates
npm audit fix --force  # Force breaking updates (review changes!)
```

**When it runs:** Pre-push hook, CI (`frontend` job).

---

## Git Hooks Architecture

### What are Git hooks?

Git hooks are scripts that run automatically at specific points in the Git workflow. They live in a directory that Git is configured to look at.

### Default vs custom hooks directory

By default, Git looks for hooks in `.git/hooks/`. This directory is **not tracked** by Git (it's inside `.git/`), so hooks can't be shared.

We use a **custom hooks directory** checked into the repo:

```
.githooks/
├── pre-commit    # Runs before every commit
└── pre-push      # Runs before every push
```

Git is told to use this directory via:

```sh
git config core.hooksPath .githooks
```

This is a **local** Git config setting (stored in `.git/config`). Each developer must run this once after cloning. The `make setup` target does this automatically.

### Hook execution

- Hooks are **shell scripts** (#!/bin/sh) — they run via Git's bundled sh.exe on Windows
- If a hook **exits with code 0** → the operation proceeds
- If a hook **exits with non-zero** → the operation is **aborted**
- `set -e` at the top means any failing command immediately stops the hook

### `.githooks/pre-commit`

Runs **before every `git commit`**. Designed to be fast.

```
┌─────────────────────────────────────────────┐
│  git commit -m "my changes"                 │
│                                             │
│  ┌──────────────────────────────────────┐   │
│  │  1. cargo fmt --all -- --check       │   │
│  │     Are all Rust files formatted?    │   │
│  │                                      │   │
│  │  2. cargo clippy ... -D warnings     │   │
│  │     Any lint warnings?               │   │
│  │                                      │   │
│  │  3. svelte-check (if web/ changed)   │   │
│  │     Any TypeScript errors?           │   │
│  └──────────────────────────────────────┘   │
│                                             │
│  All passed? → commit created               │
│  Any failed? → commit BLOCKED               │
└─────────────────────────────────────────────┘
```

**Smart frontend detection:** The hook only runs `svelte-check` if files under `web/` are staged:

```sh
if git diff --cached --name-only | grep -q '^web/'; then
    # ... run svelte-check
fi
```

This avoids wasting time on frontend checks when you only changed Rust code.

### `.githooks/pre-push`

Runs **before every `git push`**. This is the full CI-equivalent suite.

```
┌─────────────────────────────────────────────┐
│  git push origin main                       │
│                                             │
│  ┌──────────────────────────────────────┐   │
│  │  1. cargo fmt --check                │   │
│  │  2. cargo clippy                     │   │
│  │  3. cargo test --workspace           │   │
│  │  4. cargo audit                      │   │
│  │  5. svelte-check + vite build        │   │
│  │  6. npm audit --audit-level=high     │   │
│  └──────────────────────────────────────┘   │
│                                             │
│  All passed? → push proceeds                │
│  Any failed? → push BLOCKED                 │
└─────────────────────────────────────────────┘
```

---

## Makefile Targets

The `Makefile` provides convenient shortcuts for running checks manually:

| Target | Command | Purpose |
|--------|---------|---------|
| `make setup` | `git config core.hooksPath .githooks` | Enable git hooks (run once after clone) |
| `make check` | `cargo check --workspace --all-targets` | Fast type check (no codegen) |
| `make fmt` | `cargo fmt --all` | Auto-format all Rust code |
| `make fmt-check` | `cargo fmt --all -- --check` | Check formatting without modifying |
| `make lint` | `cargo clippy ... -D warnings` | Run clippy linter |
| `make test` | `cargo test --workspace` | Run all tests |
| `make audit` | `cargo audit` | Check for Rust vulnerabilities |
| `make web-install` | `cd web && npm ci` | Install frontend dependencies |
| `make web-check` | `cd web && npm run check && npm run build` | Type check + build frontend |
| `make web-audit` | `cd web && npm audit --audit-level=high` | Check for npm vulnerabilities |
| `make ci` | All of the above combined | Full local CI run |

### Using `make ci` manually

```sh
make ci
```

This runs the exact same checks as the pre-push hook and GitHub Actions CI. Use it anytime you want to verify everything without actually pushing.

---

## Setup Instructions

### First-time setup after cloning

```sh
# 1. Clone the repo
git clone https://github.com/signalrust/modelsentry.git
cd modelsentry

# 2. Install Rust (if not installed)
# https://rustup.rs

# 3. Install cargo-audit
cargo install cargo-audit

# 4. Windows only: install GNU toolchain + MinGW gcc
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup override set stable-x86_64-pc-windows-gnu
# Install MSYS2 (https://www.msys2.org), then:
# pacman -S mingw-w64-x86_64-gcc
# Add C:\msys64\mingw64\bin to your PATH

# 5. Install Node.js 22+ (for frontend)
# https://nodejs.org

# 6. Install frontend dependencies
make web-install

# 7. Activate git hooks
make setup
```

### Verifying the setup

```sh
# Should complete without errors:
make ci
```

---

## Workflow: What Happens When You Commit

1. You run `git commit -m "feat: add new endpoint"`
2. Git finds `.githooks/pre-commit` (because `core.hooksPath = .githooks`)
3. The hook runs as a shell script:
   - `cargo fmt --all -- --check` → ensures formatting is correct
   - `cargo clippy --workspace --all-targets -- -D warnings` → ensures no lint warnings
   - If `web/` files are staged: `svelte-check` → ensures TypeScript is valid
4. **If everything passes:** the commit is created normally
5. **If anything fails:** the commit is aborted, error message tells you what to fix

### Fixing a failed pre-commit

```sh
# Formatting failed?
cargo fmt --all
git add -u
git commit -m "..."

# Clippy failed?
# Read the error, fix the code, then re-commit

# svelte-check failed?
cd web && npm run check
# Fix TypeScript errors, then re-commit
```

---

## Workflow: What Happens When You Push

1. You run `git push origin main`
2. Git finds `.githooks/pre-push`
3. The hook runs the **full check suite:**
   - `cargo fmt --check`
   - `cargo clippy`
   - `cargo test --workspace` (runs all 192 tests)
   - `cargo audit` (checks RustSec database)
   - `svelte-check` + `vite build`
   - `npm audit --audit-level=high`
4. **If everything passes:** the push goes through to GitHub
5. **If anything fails:** the push is blocked, nothing reaches the remote

### Fixing a failed pre-push

```sh
# Tests failed?
cargo test --workspace
# Fix failing tests, commit, try pushing again

# cargo audit failed?
cargo update        # Update dependencies to patched versions
cargo audit         # Verify fix
git add Cargo.lock
git commit -m "fix(deps): update to resolve audit findings"
git push

# npm audit failed?
cd web
npm audit fix
cd ..
git add web/package-lock.json
git commit -m "fix(deps): resolve npm audit findings"
git push
```

---

## How Local Hooks Map to CI

Every check in the local hooks has an exact counterpart in GitHub Actions:

| Local Hook | CI Job | CI Step |
|------------|--------|---------|
| `cargo fmt --check` | `rust` | Fmt |
| `cargo clippy ... -D warnings` | `rust` | Clippy |
| `cargo test --workspace` | `rust` | Test |
| `cargo audit` | `security` | Cargo Audit |
| `svelte-check` | `frontend` | Type Check |
| `vite build` | `frontend` | Build |
| `npm audit --audit-level=high` | `frontend` | Audit |

The CI workflow is defined in `.github/workflows/ci.yml` and runs on every push and pull request.

---

## Troubleshooting

### "cargo fmt check failed"

```sh
cargo fmt --all    # Auto-fix, then re-add and commit
```

### "clippy found warnings"

Read the clippy output carefully — it tells you the exact file, line, and suggested fix. Clippy suggestions are almost always correct.

### "linking with link.exe failed" (Windows)

You're using the MSVC toolchain. Switch to GNU:

```sh
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup override set stable-x86_64-pc-windows-gnu
```

### "gcc.exe not found" (Windows)

Install MinGW via MSYS2:

```sh
# Install MSYS2 from https://www.msys2.org
# Then in MSYS2 terminal:
pacman -S mingw-w64-x86_64-gcc
```

Add `C:\msys64\mingw64\bin` to your system PATH.

### "cargo audit: command not found"

```sh
cargo install cargo-audit
```

### Bypassing hooks in an emergency

If you absolutely must skip hooks (e.g., trivial docs-only change):

```sh
git commit --no-verify -m "docs: ..."
git push --no-verify
```

**Use sparingly.** CI will still catch issues, but the whole point of hooks is to catch them before they hit CI.

### Hooks not running at all

```sh
# Verify hooks path is set
git config core.hooksPath
# Should output: .githooks

# If not:
make setup
```
