# lockpick

[![CI](https://github.com/Kuenlun/lockpick/actions/workflows/rust.yml/badge.svg?branch=master)](https://github.com/Kuenlun/lockpick/actions/workflows/rust.yml)
[![codecov](https://codecov.io/gh/Kuenlun/lockpick/branch/master/graph/badge.svg)](https://codecov.io/gh/Kuenlun/lockpick)
[![Crates.io](https://img.shields.io/crates/v/lockpick.svg)](https://crates.io/crates/lockpick)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

> One invocation, one truth, zero noise. Run every quality gate your Rust crate needs and ship perfect code.

`lockpick` is a single binary that orchestrates the full quality pipeline for a Rust workspace: compilation, lints, formatting, tests, documentation, dependency hygiene, security advisories, plus opt-in license-header and per-metric coverage gates. All in one command, with one summary, and one exit code.

Use it locally (pre-commit), use it in CI (one job), and get the same answer either place.

```
  check    PASS
  clippy   PASS
  fmt      PASS
  test     PASS
  doc      PASS
  doc-test PASS
  machete  PASS
  audit    PASS
  license  PASS
  coverage PASS

  OK: 10/10 checks passed
```

(That run has both opt-in gates enabled; a project without them shows the eight always-on checks.)

## Install

```sh
cargo install lockpick
```

External tools used by individual checks. Missing ones fail fast with an install hint:

- `cargo install cargo-llvm-cov`: `coverage` (only when the opt-in gate is active)
- `cargo install cargo-machete`: `machete`
- `cargo install cargo-audit`: `audit`
- `cargo install cargo-nextest --locked`: optional, auto-detected for faster `test` output

## Quick start

```sh
lockpick                          # one status line per check, FAIL sections only
lockpick -v                       # CI mode: every command, every PASS/FAIL section
lockpick --fix                    # auto-fix fmt, clippy and machete before checks
lockpick --coverage               # also enforce the coverage gate (100% defaults)
lockpick --skip audit --skip doc  # skip checks (repeatable, or comma-separated)
```

## Checks

| Check      | What it does                                                            | `--skip`   |
|------------|-------------------------------------------------------------------------|------------|
| `check`    | `cargo check` on every target and feature                               | `check`    |
| `clippy`   | `cargo clippy` with `pedantic` + `nursery` + `cargo`* and `-D warnings` | `clippy`   |
| `fmt`      | `cargo fmt --all --check`                                               | `fmt`      |
| `test`     | `cargo test`, auto-routed through `nextest` or `llvm-cov` when present  | `test`     |
| `doc`      | `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings`                   | `doc`      |
| `doc-test` | doctests, skipped on bin-only workspaces                                | `doc-test` |
| `machete`  | unused-dependency scan (`cargo machete`)                                | `machete`  |
| `audit`    | RustSec advisory scan (`cargo audit`, requires network)                 | `audit`    |
| `license`  | byte-equal license-header scan, opt-in via config                       | `license`  |
| `coverage` | per-metric `llvm-cov` gate, opt-in via config or `--coverage`           | `coverage` |

\* `clippy::multiple_crate_versions` is exempted from the `cargo` group: duplicate versions almost always come from transitive dependencies the checked project cannot fix.

`--skip test` implies `--skip coverage`. `--skip license` and `--skip coverage` are no-ops when the matching gate is not configured. Run `lockpick -v` to see the exact cargo invocation each check fires.

## Configuration

Add a `[workspace.metadata.lockpick]` (preferred) or `[package.metadata.lockpick]` block to your `Cargo.toml`. Every field is optional.

```toml
[workspace.metadata.lockpick]
skip = ["audit", "machete"]                   # same identifiers as `--skip`
license-header = ".github/license_header.rs"
# license-header-globs = ["src/**/*.rs", "tests/**/*.rs"]  # defaults shown below

# Presence of this table (even empty) enables the coverage gate.
[workspace.metadata.lockpick.coverage]
functions = 100   # every threshold defaults to 100
lines     = 100
regions   = 100
# branches = 100  # opt-in, nightly-only (fails on stable with exit 4)
```

CLI `--skip` is additive on top of the `skip` array.

The `license` check compares the start of each file to the header template. Default globs are `src/**/*.rs`, `tests/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs`. Files marked `@generated` are skipped.

The `coverage` check is opt-in: add the `[workspace.metadata.lockpick.coverage]` table (even empty) or pass `--coverage`. Once active it parses `cargo llvm-cov report --json` and enforces each threshold (100% unless configured) with exact integer comparison. Combining `--coverage` with `--skip coverage` or `--skip test` is a usage error (exit `2`). The `branches` metric is nightly-only (`rustup toolchain install nightly --component llvm-tools-preview`). On stable it is silently dropped, and an explicit `coverage.branches` aborts with exit `4`. On failure, drill in with `cargo llvm-cov --html` (`--branch` on nightly) and open `target/llvm-cov/html/index.html`.

## Exit codes

| Code | Meaning                                                                                |
|------|----------------------------------------------------------------------------------------|
| 0    | All checks passed                                                                      |
| 1    | One or more checks failed                                                              |
| 2    | Usage error (unknown flag, invalid `--skip` value, contradictory `--coverage`, or every check skipped) |
| 3    | A required external tool (`cargo-llvm-cov`, `cargo-machete`, `cargo-audit`) is absent  |
| 4    | `coverage.branches` is configured but the active toolchain is stable                   |

## Pre-commit / CI

Minimal pre-commit hook:

```sh
#!/usr/bin/env bash
lockpick || exit 1
```

Minimal GitHub Actions job:

```yaml
- uses: taiki-e/install-action@v2
  with:
    tool: lockpick,cargo-llvm-cov,cargo-machete,cargo-audit
- run: lockpick -v
```

## How it schedules

Cargo holds an exclusive lock on `target/.cargo-lock` while a build subcommand runs. lockpick schedules around it:

```text
  fmt        ┐
  machete    ├── parallel (no target/ contention)
  audit      │
  license    ┘

  check ──► test ──► clippy ──► doc ──► doc-test    serial (share target/.cargo-lock)
              │
              └──► coverage (when active)           post-test, parallel with chain tail
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
