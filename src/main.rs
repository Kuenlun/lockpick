// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

// `coverage(off)` on unit-test modules keeps `cargo llvm-cov` focused
// on production code. The cfg is injected by cargo-llvm-cov on nightly.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod checks;
mod cli;
mod config;
mod error;
mod fix;
mod reporter;
mod runner;
mod signals;
mod tooling;

use crate::error::LockpickError;

use {clap::Parser, std::process::ExitCode};

fn main() -> ExitCode {
    signals::install();
    let cli = cli::Cli::parse();

    // Meta subcommands bypass the pipeline: emit their artifact and
    // exit cleanly. No signal-aware exit code is needed since nothing
    // was spawned.
    if let Some(cli::Cmd::Completions { shell }) = &cli.command {
        cli::Cli::write_completions(*shell, &mut std::io::stdout());
        return ExitCode::SUCCESS;
    }

    let result = runner::run(cli);
    ExitCode::from(signals::exit_code(
        signals::state().captured(),
        dispatch(result),
    ))
}

/// Map a [`runner::run`] result to a process exit code: `0` success,
/// `1` check failure, `2` usage error (empty pipeline or contradictory
/// coverage flags), `3` missing tool, `4` `coverage.branches` on
/// stable. Pre-check errors print their Display to stderr.
/// `ChecksFailed` stays silent because the reporter already rendered
/// the per-check FAIL sections.
fn dispatch(result: Result<(), LockpickError>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(LockpickError::ChecksFailed(_)) => 1,
        Err(e @ (LockpickError::NoChecksToRun | LockpickError::CoverageConflict(_))) => {
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
