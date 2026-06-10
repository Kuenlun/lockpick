# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0](https://github.com/Kuenlun/lockpick/compare/v0.6.0...v0.7.0) - 2026-06-10

### Added

- [**breaking**] make coverage gate opt-in ([#64](https://github.com/Kuenlun/lockpick/pull/64))
- *(fix)* add --fix to auto-apply clippy/machete/fmt before checks ([#57](https://github.com/Kuenlun/lockpick/pull/57))
- *(cli)* accept `--skip a,b,c` comma list ([#55](https://github.com/Kuenlun/lockpick/pull/55))
- *(audit)* skip when the advisory database is unreachable ([#53](https://github.com/Kuenlun/lockpick/pull/53))
- *(cli)* add `completions <shell>` subcommand ([#43](https://github.com/Kuenlun/lockpick/pull/43))
- *(cli)* add --color flag to override TTY and NO_COLOR ([#42](https://github.com/Kuenlun/lockpick/pull/42))
- *(signals)* capture SIGINT/SIGTERM and forward to cargo children ([#41](https://github.com/Kuenlun/lockpick/pull/41))
- *(cli)* wrap --help and add Examples/Environment sections ([#40](https://github.com/Kuenlun/lockpick/pull/40))
- *(config)* accept skip = [...] in [*.metadata.lockpick] ([#39](https://github.com/Kuenlun/lockpick/pull/39))
- *(output)* forward color decision to cargo and rustfmt subprocesses ([#38](https://github.com/Kuenlun/lockpick/pull/38))
- *(reporter)* split report onto stdout, keep diagnostics on stderr ([#37](https://github.com/Kuenlun/lockpick/pull/37))

### Fixed

- *(doc)* preserve user-supplied RUSTDOCFLAGS ([#50](https://github.com/Kuenlun/lockpick/pull/50))
- *(config)* detect doc-tests across all library crate kinds ([#49](https://github.com/Kuenlun/lockpick/pull/49))
- *(coverage)* make --branch nightly-only, exit 4 when branches is set on stable ([#36](https://github.com/Kuenlun/lockpick/pull/36))
- *(config)* reject unknown keys in [package.metadata.lockpick] ([#35](https://github.com/Kuenlun/lockpick/pull/35))
- *(cli)* list every phase in --help, unify doc-test label, warn on inert --skip ([#34](https://github.com/Kuenlun/lockpick/pull/34))
- *(runner)* anchor cwd to workspace root before running checks ([#33](https://github.com/Kuenlun/lockpick/pull/33))
- *(runner)* [**breaking**] exit 2 when every check is skipped instead of 0 ([#32](https://github.com/Kuenlun/lockpick/pull/32))
- *(runner)* bundle missing-tool errors into one install hint ([#29](https://github.com/Kuenlun/lockpick/pull/29))

### Other

- *(config)* skip coverage instead of pinning thresholds to zero ([#63](https://github.com/Kuenlun/lockpick/pull/63))
- add integration suite for CLI, config and runtime contracts ([#62](https://github.com/Kuenlun/lockpick/pull/62))
- *(deps)* loosen version pins and bump signal-hook to 0.4 ([#61](https://github.com/Kuenlun/lockpick/pull/61))
- *(cli)* tighten short help and move details to long help ([#60](https://github.com/Kuenlun/lockpick/pull/60))
- refresh tagline and tighten copy across README, CLI and comments ([#59](https://github.com/Kuenlun/lockpick/pull/59))
- replace em dashes with conventional punctuation ([#58](https://github.com/Kuenlun/lockpick/pull/58))
- *(runner)* anchor cargo children via `current_dir` instead of process chdir ([#56](https://github.com/Kuenlun/lockpick/pull/56))
- *(runner)* note that colored override is process-wide ([#54](https://github.com/Kuenlun/lockpick/pull/54))
- *(checks)* split mod.rs into plan, runner, util ([#52](https://github.com/Kuenlun/lockpick/pull/52))
- *(reporter)* batch section dump under one suspend ([#51](https://github.com/Kuenlun/lockpick/pull/51))
- drop remaining test-only seams and dead doc references ([#48](https://github.com/Kuenlun/lockpick/pull/48))
- *(cli)* adopt clap-cargo styling preset ([#47](https://github.com/Kuenlun/lockpick/pull/47))
- remove tests and test-only scaffolding ([#46](https://github.com/Kuenlun/lockpick/pull/46))
- *(coverage)* set thresholds to 0 ([#45](https://github.com/Kuenlun/lockpick/pull/45))
- *(runner)* schedule independent checks beside the serial chain ([#31](https://github.com/Kuenlun/lockpick/pull/31))

## [0.6.0](https://github.com/Kuenlun/lockpick/compare/v0.5.0...v0.6.0) - 2026-05-14

### Added

- Run `cargo doc --no-deps --workspace --all-features` with `RUSTDOCFLAGS=-D warnings` as a new doc check, skippable via `--skip doc` ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Run `cargo machete` as a new check for unused dependencies, skippable via `--skip machete` ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Run `cargo audit` as a new check against the RustSec advisory database, skippable via `--skip audit` ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Scan source files in-process for byte-equality against a configured header template, with default globs `src/**/*.rs`, `tests/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs`. Skip any file whose first five lines contain `@generated`, dedupe overlapping glob matches, and canonicalize paths so the configured header file does not flag itself. Skippable via `--skip license` ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Parse `cargo llvm-cov report --json --summary-only --branch` into per-metric thresholds for `functions`, `lines`, `regions`, and `branches` (default 100 each), using integer arithmetic so the gate is exact at the equality boundary. Treat `count == 0` on an individual metric as vacuously satisfied and reject reports where every metric is zero. The coverage phase runs only when both `compile` and `test` pass, and is skippable via `--skip coverage` (implied by `--skip test`) ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Auto-detect `cargo-nextest` at startup and dispatch the test phase through `cargo test`, `cargo nextest run`, `cargo llvm-cov`, or `cargo llvm-cov nextest` depending on the detected toolchain. Append `--no-tests=pass` to every nextest invocation so empty test sets keep parity with `cargo test` from nextest 0.9.85 onward ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Load configuration from `[workspace.metadata.lockpick]` (preferred) or `[package.metadata.lockpick]` with kebab-case keys `license-header`, `license-header-globs`, and a nested `coverage` table. Warn when only the package section is set in a multi-crate workspace ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Print a banner of every planned cargo invocation up front under `--verbose` ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- End every run with an `OK: N/N checks passed` (green) or `Failed: K/N (labels)` (red) summary footer ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Print a `--skip test implies coverage will be skipped` notice that stays visible even without `--verbose` ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Scrub `CARGO_PKG_*`, `CARGO_BIN_*`, `CARGO_CRATE_*`, `CARGO_MANIFEST_DIR`, `CARGO_MANIFEST_PATH`, and `CARGO_PRIMARY_PACKAGE` from every spawned cargo invocation, and detect optional cargo subcommands by scanning `PATH` for the `cargo-<sub>` binary instead of spawning `cargo <sub> --version`, so children launched under `cargo run` no longer inherit the parent crate's context. In particular this fixes `cargo-machete`'s positional-paths fallback under a leaked `CARGO_PKG_NAME` ([#27](https://github.com/Kuenlun/lockpick/pull/27))

### Changed

- **BREAKING:** Run coverage by default. Configure per-metric thresholds (default 100 for `functions`, `lines`, `regions`, and `branches`) through `[*.metadata.lockpick.coverage]`, and opt out with `--skip coverage` (implied by `--skip test`) ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- **BREAKING:** Demote `--verbose` (`-v`) from a repeat-count `u8` to a plain `bool`, so `-vv`, `-vvv`, and `-vvvv` are no longer accepted ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- **BREAKING:** Exit with code `3` and an install hint when a required cargo subcommand (`cargo-llvm-cov`, `cargo-machete`, `cargo-audit`, `cargo-nextest`) is missing, code `1` when any check fails, and code `2` on usage errors ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- **BREAKING:** Tighten the `clippy` check to enable the `pedantic`, `nursery`, and `cargo` lint groups with `-D warnings` ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- **BREAKING:** Pass `--all` to `cargo fmt --check` so every workspace member is validated ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Finish each parallel check's spinner from inside its worker thread so `PASS`/`FAIL` marks land progressively rather than all together once the slowest task completes ([#27](https://github.com/Kuenlun/lockpick/pull/27))
- Propagate panicking checks via `std::panic::resume_unwind` instead of masking them as `Fail` with empty output ([#27](https://github.com/Kuenlun/lockpick/pull/27))

### Removed

- **BREAKING:** Drop the `--coverage`/`-c` opt-in flag and the `--min-coverage` percentage threshold. Coverage now runs by default and is configured through `[*.metadata.lockpick.coverage]` ([#27](https://github.com/Kuenlun/lockpick/pull/27))

## [0.5.0](https://github.com/Kuenlun/lockpick/compare/v0.4.1...v0.5.0) - 2026-05-10

### Changed

- Relicense the crate from `GPL-3.0-or-later` to the standard Rust dual license `MIT OR Apache-2.0`. Replace per-file GPL boilerplate with a 3-line `SPDX-License-Identifier: MIT OR Apache-2.0` header across the source tree, swap the single `LICENSE` file for `LICENSE-MIT` and `LICENSE-APACHE`, and add a License section to README.md covering the inbound contribution clause ([#25](https://github.com/Kuenlun/lockpick/pull/25))

### Fixed

- *(runner)* Fall back to plain `cargo test` and mark the coverage gate as `Skip` (with a warning) when `cargo-llvm-cov` is not installed, instead of letting the child process fail opaquely ([#23](https://github.com/Kuenlun/lockpick/pull/23))
- *(runner)* Redirect child cargo invocations to `CARGO_TARGET_DIR=target/lockpick` when `lockpick` is launched via `cargo run` (detected by checking whether the running binary lives under the project's `target/` directory), avoiding build contention with the parent and "Access denied" rebuild failures on Windows where the running `.exe` is locked by the OS ([#23](https://github.com/Kuenlun/lockpick/pull/23))

## [0.4.1](https://github.com/Kuenlun/lockpick/compare/v0.4.0...v0.4.1) - 2026-04-05

### Fixed

- *(output)* bypass indicatif buffering in non-TTY environments ([#20](https://github.com/Kuenlun/lockpick/pull/20))

### Other

- *(runner)* overhaul Reporter to support non-TTY output and in-place spinner completion ([#18](https://github.com/Kuenlun/lockpick/pull/18))

## [0.4.0](https://github.com/Kuenlun/lockpick/compare/v0.3.3...v0.4.0) - 2026-04-02

### Added

- Run `cargo check` as a sequential gate before all parallel tasks, skipping remaining checks on failure ([#16](https://github.com/Kuenlun/lockpick/pull/16))
- Display color-coded output sections for task results (`✔ OUTPUT`/`✖ ERRORS`) in verbose mode ([#16](https://github.com/Kuenlun/lockpick/pull/16))

### Changed

- Replace `--check` opt-in flag with `--skip check`, making `cargo check` run by default ([#16](https://github.com/Kuenlun/lockpick/pull/16))
- Capture and display subprocess stdout/stderr inline instead of discarding output ([#16](https://github.com/Kuenlun/lockpick/pull/16))

### Fixed

- Route log output through `MultiProgress` to prevent spinner corruption ([#16](https://github.com/Kuenlun/lockpick/pull/16))
- Suppress redundant error output when checks fail ([#16](https://github.com/Kuenlun/lockpick/pull/16))

### Other

- Replace `env_logger` with a custom lightweight logger, removing 12 transitive dependencies ([#16](https://github.com/Kuenlun/lockpick/pull/16))
- replace source compilation of lockpick with prebuilt binary installation ([#14](https://github.com/Kuenlun/lockpick/pull/14))

## [0.3.3](https://github.com/Kuenlun/lockpick/compare/v0.3.2...v0.3.3) - 2026-04-02

### Other

- fix tag parsing in release-plz workflow ([#12](https://github.com/Kuenlun/lockpick/pull/12))

## [0.3.2](https://github.com/Kuenlun/lockpick/compare/v0.3.1...v0.3.2) - 2026-04-02

### Other

- fix release-plz PRs not triggering CI workflows ([#11](https://github.com/Kuenlun/lockpick/pull/11))
- fix release binary uploads by merging artifact workflow into release-plz ([#9](https://github.com/Kuenlun/lockpick/pull/9))

## [0.3.1](https://github.com/Kuenlun/lockpick/compare/v0.3.0...v0.3.1) - 2026-04-02

### Added

- *(cli)* replace opt-out flags with composable `--skip` and add coverage support ([#7](https://github.com/Kuenlun/lockpick/pull/7))

### Other

- add GitHub Actions workflow for releasing compiled binaries ([#6](https://github.com/Kuenlun/lockpick/pull/6))

## [0.3.0](https://github.com/Kuenlun/lockpick/releases/tag/v0.3.0) - 2026-04-02

### Added

- *(core)* implement core CLI logic and parallel cargo check runner ([#2](https://github.com/Kuenlun/lockpick/pull/2))
- initialize lockpick Rust project and CI infrastructure ([#1](https://github.com/Kuenlun/lockpick/pull/1))

### Other

- configure workflows and align CI with repository conventions ([#4](https://github.com/Kuenlun/lockpick/pull/4))
- configure release automation, pre-commit hooks, and CI improvements ([#3](https://github.com/Kuenlun/lockpick/pull/3))
