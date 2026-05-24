// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

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

    // Meta subcommands (currently only `completions <SHELL>`) bypass the
    // pipeline: emit their artifact to stdout and exit cleanly, no
    // signal-aware exit-code dance needed because nothing was spawned.
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

/// Map a [`runner::run`] result to a process exit code: `0` on success,
/// `2` on misconfiguration (empty pipeline), `3` on missing-tool errors,
/// `4` when `coverage.branches` is set on stable, `1` otherwise.
/// Variants that surface before any check ran echo their Display to
/// stderr; `ChecksFailed` is silent because the reporter has already
/// rendered the per-check FAIL sections.
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
