// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod checks;
mod cli;
mod config;
mod error;
mod reporter;
mod runner;
mod tooling;

use crate::error::LockpickError;

// `main` is excluded from test builds so libtest is the sole entry point
// and an unused `fn main` does not drag coverage below 100%. Functions
// only reachable from `main` carry `#[cfg_attr(test, allow(dead_code))]`.
#[cfg(not(test))]
use {clap::Parser, std::process::ExitCode};

#[cfg(not(test))]
fn main() -> ExitCode {
    ExitCode::from(dispatch(runner::run(&cli::Cli::parse())))
}

/// Map a [`runner::run`] result to a process exit code: `0` on success,
/// `2` on misconfiguration (empty pipeline), `3` on missing-tool errors,
/// `4` when `coverage.branches` is set on stable, `1` otherwise.
/// Variants that surface before any check ran echo their Display to
/// stderr; `ChecksFailed` is silent because the reporter has already
/// rendered the per-check FAIL sections.
#[cfg_attr(test, allow(dead_code))]
fn dispatch(result: Result<(), LockpickError>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(LockpickError::ChecksFailed(_)) => 1,
        Err(e @ LockpickError::NoChecksToRun) => {
            eprintln!("error: {e}");
            2
        }
        Err(e @ LockpickError::MissingTools(_)) => {
            eprintln!("error: {e}");
            3
        }
        Err(e @ LockpickError::BranchesRequireNightly) => {
            eprintln!("error: {e}");
            4
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::error::MissingTool;

    #[test]
    fn dispatch_maps_ok_to_zero() {
        assert_eq!(dispatch(Ok(())), 0);
    }

    #[test]
    fn dispatch_maps_checks_failed_to_one() {
        assert_eq!(dispatch(Err(LockpickError::ChecksFailed(2))), 1);
    }

    #[test]
    fn dispatch_maps_no_checks_to_run_to_two() {
        assert_eq!(dispatch(Err(LockpickError::NoChecksToRun)), 2);
    }

    #[test]
    fn dispatch_maps_missing_tools_to_three() {
        let missing = vec![MissingTool {
            binary: "cargo-llvm-cov",
            skip_flag: "coverage",
        }];
        assert_eq!(dispatch(Err(LockpickError::MissingTools(missing))), 3);
    }

    #[test]
    fn dispatch_maps_branches_require_nightly_to_four() {
        // Covers the arm on non-unix targets, where the integration test
        // `coverage_branches_on_stable_exits_with_four_and_actionable_hint`
        // is gated by `#[cfg(unix)]`.
        assert_eq!(dispatch(Err(LockpickError::BranchesRequireNightly)), 4);
    }
}
