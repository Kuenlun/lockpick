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
/// `3` on missing-tool errors, `1` otherwise.
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
