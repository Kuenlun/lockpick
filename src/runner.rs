/*!
lockpick - Rust CLI to enforce merge checks and code quality
Copyright (C) 2026  Juan Luis Leal Contreras (Kuenlun)

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

use crate::cli::Cli;
use crate::error::LockpickError;
use std::process::{Command, Stdio};

pub fn run(_cli: Cli) -> Result<(), LockpickError> {
    let common_args = ["--workspace", "--all-targets", "--all-features"];

    run_cargo("check", &common_args)?;
    run_cargo("clippy", &common_args)?;
    run_cargo("fmt", &["--check"])?;
    run_cargo("test", &common_args)?;

    Ok(())
}

fn run_cargo(subcommand: &str, args: &[&str]) -> Result<(), LockpickError> {
    log::info!("Running cargo {subcommand}");

    let verbose = log::log_enabled!(log::Level::Info);
    let stdio = || {
        if verbose {
            Stdio::inherit()
        } else {
            Stdio::null()
        }
    };

    let status = Command::new("cargo")
        .arg(subcommand)
        .args(args)
        .stdout(stdio())
        .stderr(stdio())
        .status()?;

    if status.success() {
        return Ok(());
    }

    Err(LockpickError::CargoFailed {
        subcommand: subcommand.to_owned(),
        status,
    })
}
