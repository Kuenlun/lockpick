// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

mod checks;
mod cli;
mod config;
mod error;
mod reporter;
mod runner;
mod tooling;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;
use crate::error::LockpickError;

fn main() -> ExitCode {
    ExitCode::from(dispatch(runner::run(&Cli::parse())))
}

/// Translate a [`runner::run`] result into an exit code while emitting any
/// user-facing diagnostics. Exit codes follow the convention used by other
/// Rust CLIs: `0` on success, `3` for missing-tool errors so wrappers can
/// distinguish "lockpick couldn't run" from "lockpick ran and a check
/// failed", and `1` for everything else.
fn dispatch(result: Result<(), LockpickError>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(LockpickError::ChecksFailed(_)) => 1,
        Err(e @ LockpickError::MissingTool { .. }) => {
            eprintln!("error: {e}");
            3
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn dispatch_returns_zero_on_success() {
        assert_eq!(dispatch(Ok(())), 0);
    }

    #[test]
    fn dispatch_returns_one_for_checks_failed() {
        assert_eq!(dispatch(Err(LockpickError::ChecksFailed(2))), 1);
    }

    #[test]
    fn dispatch_returns_three_for_missing_tool() {
        let err = LockpickError::MissingTool {
            tool: "cargo-llvm-cov",
            install: "cargo install cargo-llvm-cov",
        };
        assert_eq!(dispatch(Err(err)), 3);
    }

    #[test]
    fn dispatch_returns_one_for_io_error() {
        let err = LockpickError::Io(io::Error::other("boom"));
        assert_eq!(dispatch(Err(err)), 1);
    }
}
