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

mod cli;
mod error;
mod logger;
mod runner;

use clap::Parser;

use crate::cli::{Cli, SkipOption};
use crate::error::LockpickError;

fn main() -> Result<(), LockpickError> {
    let cli = Cli::parse();

    if cli.opt_in.check && cli.skips(&SkipOption::Check) {
        eprintln!("error: --check and --skip check are mutually exclusive");
        std::process::exit(2);
    }

    if cli.opt_in.coverage && cli.skips(&SkipOption::Test) {
        eprintln!("error: --coverage and --skip test are mutually exclusive");
        std::process::exit(2);
    }

    logger::init(cli.verbose);
    runner::run(&cli)
}
