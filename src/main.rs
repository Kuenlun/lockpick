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

// `main` and its production-only imports are skipped in test builds so
// libtest's harness is the only entry point — otherwise rustc emits an
// unused instantiation of `fn main` that drops the binary's coverage below
// 100% even though every line is exercised by the production binary the
// integration tests spawn. Functions only reachable from `main`
// (`dispatch`, `runner::run`, `Toolchain::detect`, `CargoCli::detect`)
// carry a matching `#[cfg_attr(test, allow(dead_code))]`.
#[cfg(not(test))]
use {clap::Parser, std::process::ExitCode};

#[cfg(not(test))]
fn main() -> ExitCode {
    ExitCode::from(dispatch(runner::run(&cli::Cli::parse())))
}

/// Translate a [`runner::run`] result into an exit code while emitting any
/// user-facing diagnostics. Exit codes follow the convention used by other
/// Rust CLIs: `0` on success, `3` for missing-tool errors so wrappers can
/// distinguish "lockpick couldn't run" from "lockpick ran and a check
/// failed", and `1` for everything else. The match is exhaustive over
/// every variant of [`LockpickError`]; integration tests drive all three
/// arms via the production binary, so no unit tests live here.
#[cfg_attr(test, allow(dead_code))]
fn dispatch(result: Result<(), LockpickError>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(LockpickError::ChecksFailed(_)) => 1,
        Err(e @ LockpickError::MissingTool { .. }) => {
            eprintln!("error: {e}");
            3
        }
    }
}
