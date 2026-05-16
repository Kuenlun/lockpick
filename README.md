# lockpick

[![CI](https://github.com/Kuenlun/lockpick/actions/workflows/rust.yml/badge.svg?branch=master)](https://github.com/Kuenlun/lockpick/actions/workflows/rust.yml)
[![codecov](https://codecov.io/gh/Kuenlun/lockpick/branch/master/graph/badge.svg)](https://codecov.io/gh/Kuenlun/lockpick)
[![Crates.io](https://img.shields.io/crates/v/lockpick.svg)](https://crates.io/crates/lockpick)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

> One invocation, one truth, zero noise. Run every quality gate your Rust crate needs and ship perfect code.

`lockpick` is a single binary that orchestrates the full quality pipeline for a Rust workspace: compilation, lints, formatting, tests, documentation, dependency hygiene, security advisories, license headers and per-metric coverage. All in one command, with one summary, and one exit code.

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

## Install

```sh
cargo install lockpick
```

External tooling lockpick drives (install whichever checks you enable; missing tools fail-fast with a clear error):

- `cargo install cargo-llvm-cov`: coverage gate (on by default)
- `cargo install cargo-machete`: unused-dependency detection
- `cargo install cargo-audit`: RustSec advisory scan
- `cargo install cargo-nextest --locked`: optional, auto-detected for faster test output

Coverage measures functions, lines and regions on any toolchain. Branch coverage relies on `-Z coverage-options=branch` and only runs on nightly (`rustup toolchain install nightly --component llvm-tools-preview`). On stable the branches metric is dropped with a visible note, and setting `coverage.branches` in config is rejected with exit `4`.

## Quick start

```sh
lockpick                          # one status line per check, FAIL sections only
lockpick -v                       # CI mode: cargo banner + every PASS/FAIL section
lockpick --skip audit --skip doc  # skip checks (repeatable)
```

## Checks

| Check      | What it does                                                                | `--skip`   |
|------------|-----------------------------------------------------------------------------|------------|
| `check`    | `cargo check` on every target and feature                                   | `check`    |
| `clippy`   | `cargo clippy` with `pedantic` + `nursery` + `cargo` and `-D warnings`      | `clippy`   |
| `fmt`      | `cargo fmt --all --check`                                                   | `fmt`      |
| `test`     | `cargo test` / `nextest` / `llvm-cov`, auto-routed by what is installed and whether coverage is active | `test`     |
| `doc`      | `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings`                       | `doc`      |
| `doc-test` | doctests; skipped on bin-only workspaces                                    | `doc-test` |
| `machete`  | unused-dependency scan (`cargo machete`)                                    | `machete`  |
| `audit`    | RustSec advisory scan (`cargo audit`, requires network)                     | `audit`    |
| `license`  | byte-equal license-header scan; opt-in via config                           | `license`  |
| `coverage` | per-metric `llvm-cov` gate, runs once `test` passes                         | `coverage` |

`--skip test` implies `--skip coverage`. `--skip license` is a no-op when no header is configured. Run `lockpick -v` to see the exact cargo invocation each check fires.

## Configuration

Add a `[workspace.metadata.lockpick]` (preferred) or `[package.metadata.lockpick]` block to your `Cargo.toml`. Every field is optional.

```toml
[workspace.metadata.lockpick]
license-header = ".github/license_header.rs"
# license-header-globs = ["src/**/*.rs", "tests/**/*.rs"]  # defaults shown below

[workspace.metadata.lockpick.coverage]
functions = 100   # functions, lines and regions default to 100
lines     = 100
regions   = 100
# branches = 100  # opt-in, nightly-only (fails on stable with exit 4)
```

The license check reads the header file, walks the globs (default: `src/**/*.rs`, `tests/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs`), skips files marked `@generated`, and lists every offender. The coverage check parses the JSON from `cargo llvm-cov report`, treats `count == 0` as vacuously satisfied, rejects all-zero entries as broken instrumentation, and points at `cargo llvm-cov --html` on failure (with `--branch` on nightly).

## Exit codes

| Code | Meaning                                                                                |
|------|----------------------------------------------------------------------------------------|
| 0    | All checks passed                                                                      |
| 1    | One or more checks failed                                                              |
| 2    | Usage error (unknown flag, invalid `--skip` value, or every check skipped via `--skip`)|
| 3    | A required external tool (`cargo-llvm-cov`, `cargo-machete`, `cargo-audit`) is absent  |
| 4    | `coverage.branches` is configured but the active toolchain is stable                   |

## Pre-commit / CI

A minimal pre-commit hook:

```sh
#!/usr/bin/env bash
lockpick || exit 1
```

A minimal GitHub Actions job:

```yaml
- uses: taiki-e/install-action@v2
  with:
    tool: lockpick,cargo-llvm-cov,cargo-machete,cargo-audit
- run: lockpick -v
```

## How it schedules

Cargo holds an exclusive lock on `target/.cargo-lock` while any subcommand mutates the build directory. lockpick schedules around that constraint:

```text
  fmt        ┐
  machete    ├── parallel (no target/ contention)
  audit      │
  license    ┘

  check ──► test ──► clippy ──► doc ──► doc-test    serial (share target/.cargo-lock)
              │
              └──► coverage                         post-test
```

The independent cohort runs alongside the serial chain. Coverage forks off the chain as soon as `test` finishes and runs in parallel with the chain tail.

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
