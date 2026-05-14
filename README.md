# lockpick

[![CI](https://github.com/Kuenlun/lockpick/actions/workflows/rust.yml/badge.svg?branch=master)](https://github.com/Kuenlun/lockpick/actions/workflows/rust.yml)
[![codecov](https://codecov.io/gh/Kuenlun/lockpick/branch/master/graph/badge.svg)](https://codecov.io/gh/Kuenlun/lockpick)
[![Crates.io](https://img.shields.io/crates/v/lockpick.svg)](https://crates.io/crates/lockpick)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

> One invocation, one truth, zero noise. Run every quality gate your Rust crate needs and ship perfect code.

`lockpick` is a single binary that orchestrates the full quality pipeline for a Rust workspace: compilation, lints, formatting, tests, documentation, dependency hygiene, security advisories, license headers and 100% branch coverage — all in one command, with one summary, and one exit code.

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

- `cargo install cargo-llvm-cov` — coverage gate (on by default)
- `cargo install cargo-machete` — unused-dependency detection
- `cargo install cargo-audit` — RustSec advisory scan
- `cargo install cargo-nextest --locked` — optional, auto-detected for faster test output

A nightly toolchain is required for branch coverage (`rustup toolchain install nightly --component llvm-tools-preview`).

## Quick start

From your crate root:

```sh
lockpick           # runs everything, default output
lockpick -v        # CI mode: command banner + every section shown
```

Skip a specific check (repeatable):

```sh
lockpick --skip audit --skip coverage
```

## Checks

| Label      | Subcommand                                | Notes                                                |
|------------|-------------------------------------------|------------------------------------------------------|
| check      | `cargo check --workspace --all-targets --all-features` | Sequential gate; on failure, parallel checks are skipped to surface compile errors cleanly. |
| clippy     | `cargo clippy --workspace --all-targets --all-features` | Parallel.                                            |
| fmt        | `cargo fmt --check`                       | Parallel.                                            |
| test       | `cargo test` / `cargo nextest run` / `cargo llvm-cov [nextest] --branch --no-report` | Parallel. Auto-uses nextest if installed; auto-instruments when coverage is active. |
| doc        | `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace --all-features` | Parallel. Catches broken intra-doc links. |
| doc-test   | `cargo test --doc --workspace --all-features` | Parallel. Skipped on bin-only workspaces.            |
| machete    | `cargo machete`                           | Parallel. Detects unused dependencies.               |
| audit      | `cargo audit`                             | Parallel. Scans the lockfile against RustSec.        |
| license    | byte-equal header check                   | Parallel. Opt-in via `[*.metadata.lockpick]`.        |
| coverage   | `cargo llvm-cov report --json --summary-only --branch` | Post-test phase. Validates functions, lines, regions, branches against per-metric thresholds. |

## Configuration

Drop a `[workspace.metadata.lockpick]` (preferred) or `[package.metadata.lockpick]` block into your `Cargo.toml`. Everything is optional and falls back to sensible defaults.

```toml
[workspace.metadata.lockpick]
license-header = ".github/license_header.rs"
# Default globs cover src/, tests/, examples/ and benches/.
# license-header-globs = ["src/**/*.rs", "tests/**/*.rs"]

[workspace.metadata.lockpick.coverage]
functions = 100
lines     = 100
regions   = 100
branches  = 100
# Omitted metrics keep their default of 100.
```

The license-header check:

- Reads the canonical header file as bytes.
- Walks the configured globs (default: `src/**/*.rs`, `tests/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs`).
- Skips files whose first line contains `@generated` (Buf/Prost convention).
- Reports every offending path, not just the first one.

The coverage check:

- Runs after `test` finishes successfully.
- Parses the JSON summary from `cargo llvm-cov report`.
- Treats `count == 0` on a metric as vacuously satisfied (a crate without conditional branches has 0/0 branches and that is fine).
- Rejects entries where *every* metric reports `count == 0` — that pattern signals broken instrumentation or no tests collected.
- On failure, points you at `cargo llvm-cov --branch --html` for the HTML report.

## Skipping checks

| Value      | What it skips                                            |
|------------|-----------------------------------------------------------|
| `check`    | `cargo check` gate                                        |
| `clippy`   | lints                                                     |
| `fmt`      | formatting                                                |
| `test`     | tests (implicitly skips `coverage` too)                   |
| `doc-test` | doc tests                                                 |
| `doc`      | `cargo doc`                                               |
| `machete`  | unused-dependency scan                                    |
| `audit`    | RustSec advisory scan                                     |
| `license`  | license-header check (silent skip when not configured)    |
| `coverage` | coverage gate                                             |

`--skip` is repeatable: `lockpick --skip audit --skip machete`.

## Exit codes

| Code | Meaning                                                            |
|------|--------------------------------------------------------------------|
| 0    | All checks passed.                                                 |
| 1    | One or more checks failed.                                         |
| 2    | Usage error (unknown flag, unknown `--skip` value, etc.).          |
| 3    | A required external tool (`cargo-llvm-cov`, `cargo-machete`, `cargo-audit`) is not installed. |

## Output

Default output is one status line per check, plus the FAIL sections only:

```
  check    PASS
  clippy   PASS
  fmt      FAIL
  test     PASS
  ...

 ✖ FMT ERRORS
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 │ Diff in src/runner.rs:42 …
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  Failed: 1/N (fmt)
```

`-v / --verbose` adds a banner listing every cargo invocation up front and shows the PASS sections too (`✔ CHECK OUTPUT`, …). This is the mode to enable in CI logs.

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
