// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

mod checks;
mod cli;
mod config;
mod error;
mod logger;
mod reporter;
mod runner;
mod tooling;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, SkipOption};
use crate::error::LockpickError;

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.opt_in.coverage && cli.skips(&SkipOption::Test) {
        eprintln!("error: --coverage and --skip test are mutually exclusive");
        std::process::exit(2);
    }

    match runner::run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(LockpickError::ChecksFailed(_)) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
