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
| `audit`    | RustSec advisory scan (`cargo audit --deny warnings`, requires network)                 | `audit`    |
| `license`  | byte-equal license-header scan, opt-in via config                       | `license`  |
| `targets`  | additional Clippy checks for configured targets and profiles           | `targets` |
| `coverage` | per-metric `llvm-cov` gate, opt-in via config or `--coverage`           | `coverage` |

\* `clippy::multiple_crate_versions` is exempted from the `cargo` group: duplicate versions almost always come from transitive dependencies the checked project cannot fix.

The audit gate fails on advisory warnings and on tool or network errors. An unavailable advisory database is not a successful security check. To intentionally omit the gate, use `--skip audit`.

The documentation gate appends `-D warnings` to existing `RUSTDOCFLAGS`, or to `CARGO_ENCODED_RUSTDOCFLAGS` when present, preserving Cargo's encoded-flag precedence and argument boundaries.


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

CLI `--skip` is additive on top of the `skip` array. Workspace metadata takes precedence over package metadata. Package metadata is accepted only in single-package workspaces. Invalid or ambiguous configuration and failed metadata discovery stop the run with exit `2`, before checks or fixes. Coverage thresholds must be integers from 0 to 100.

The `license` check compares the start of each file to the header template. The template and explicit globs are relative to the workspace root, including when Lockpick starts in a member directory. Default globs are `src/**/*.rs`, `tests/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs` in every workspace package. Default roots may be absent, but every explicitly configured pattern must match at least one source file. Invalid patterns, traversal errors and a scan matching no source files fail the gate. Files marked `@generated` are skipped.

The `coverage` check is opt-in: add the `[workspace.metadata.lockpick.coverage]` table (even empty) or pass `--coverage`. Each instrumented run first executes `cargo llvm-cov clean --workspace`, so an earlier passing run cannot supply coverage for tests that no longer exercise the code. Cleanup failure stops the test and coverage gates. Once active it parses `cargo llvm-cov report --json` and enforces each threshold (100% unless configured) with exact integer comparison. Combining `--coverage` with `--skip coverage` or `--skip test` is a usage error (exit `2`). The `branches` metric is nightly-only (`rustup toolchain install nightly --component llvm-tools-preview`). On stable it is not measured, and an explicit `coverage.branches` aborts with exit `4`. Malformed reports, missing required metrics, impossible counts, empty files and all-zero totals fail the gate. On failure, inspect the existing measurement with `cargo llvm-cov report --html` (`--branch` on nightly) and open `target/llvm-cov/html/index.html`.

## Host tests and embedded targets

Additional checks run Clippy, which also checks compilation, without executing tests or linking an executable. They use the same strict lint policy as the ordinary Clippy gate. Default gates remain enabled.

```toml
[workspace.metadata.lockpick]
locked = true       # fail if Cargo.lock is absent or needs updating
host-target = true  # use --target host-tuple despite Cargo's build.target

[[workspace.metadata.lockpick.target-checks]]
target = "thumbv8m.main-none-eabihf"
artifacts = "lib"

[[workspace.metadata.lockpick.target-checks]]
target = "thumbv8m.main-none-eabihf"
artifacts = "lib"
profile = "release"
```

Install the target with `rustup target add thumbv8m.main-none-eabihf`. This example executes ordinary tests on the host and checks embedded libraries in both dev and release profiles. Additional commands run serially under one `targets` result. A failure in one profile remains a failure even if a later profile passes. `--skip targets` disables the additional checks.

Each entry accepts:

| Field | Default | Meaning |
|-------|---------|---------|
| `target` | ordinary gate target | Cargo triple or custom target JSON path, relative to the workspace |
| `profile` | `"dev"` | Cargo profile, including `"release"` or a custom profile |
| `artifacts` | `"all-targets"` | `"lib"`, `"bins"`, or `"all-targets"` |
| `packages` | `[]` | Empty checks the workspace, otherwise select package names |
| `all-features` | `true` | Enable every feature |
| `features` | `[]` | Explicit Cargo features, requiring `all-features = false` |
| `no-default-features` | `false` | Disable defaults, requiring `all-features = false` |

`locked` and `host-target` default to false, preserving existing Cargo behavior. They also apply to ordinary checks, tests, documentation and Clippy fixes. An explicit additional `target` overrides `host-target` for that entry only. Equal normalized entries run once, and entries identical to an enabled ordinary Clippy check are omitted. Package and feature lists are sorted and deduplicated before scheduling. Cargo remains responsible for target installation, feature existence and profile validation.

For projects that cannot execute any tests locally, configure `skip = ["test", "doc-test"]`. Coverage follows the existing test-skip policy. If the workspace cannot compile on the host at all, explicitly skip the ordinary host compilation, Clippy and documentation gates and configure the required target checks. Additional checks never silently disable host tests or any other gate.

Lockpick does not produce release artifacts. Clippy checks types and lints for the selected target/profile, but does not prove final code generation, linking, linker-script correctness, or firmware size. Keep the necessary build as a separate step:

```sh
lockpick
cargo build --workspace --all-features --lib --locked --release --target thumbv8m.main-none-eabihf
```

For a firmware executable, select `artifacts = "bins"` (and `packages` if needed), then build the required binary. This keeps artifact production, linker configuration, flashing and size inspection in Cargo or the firmware workflow.

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

On Unix, SIGINT and SIGTERM are forwarded to Cargo process groups, including their descendants. Later commands are not launched after interruption. Exit codes remain `128 + signal`. When the running Lockpick binary is inside Cargo's configured target directory, child builds use an isolated subdirectory, including custom `CARGO_TARGET_DIR` and `build.target-dir` configurations.

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

Development and release builds use Rust 1.98.1, pinned in `rust-toolchain.toml`. CI runs `lockpick --coverage` with nightly-2026-09-14 on Linux, macOS and Windows. Nightly is needed only for branch instrumentation and excluding test modules. No minimum supported Rust version is declared.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
