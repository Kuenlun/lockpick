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

use clap::{
    ArgAction, Args, Parser, ValueEnum,
    builder::styling::{AnsiColor, Effects, Styles},
};

/// Steps that can be skipped via `--skip`
#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum SkipOption {
    Clippy,
    Test,
    DocTest,
    Fmt,
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Rust merge check CLI to enforce successful build, \
             formatting, Clippy lints, passing tests and code coverage",
    long_about = None,
    styles = cli_styles()
)]
pub struct Cli {
    #[command(flatten)]
    pub opt_in: OptInFlags,

    /// Skip one or more checks (e.g. --skip clippy --skip fmt)
    #[arg(long, value_enum)]
    pub skip: Vec<SkipOption>,

    #[arg(
        short = 'v',
        long = "verbose",
        action = ArgAction::Count,
        help = "Increase logging verbosity (..= -vvvv)"
    )]
    pub verbose: u8,
}

impl Cli {
    pub fn skips(&self, option: &SkipOption) -> bool {
        self.skip.contains(option)
    }
}

#[derive(Args, Debug)]
pub struct OptInFlags {
    /// Measure and enforce code coverage
    #[arg(short = 'c', long)]
    pub coverage: bool,

    /// Minimum line coverage percentage (requires --coverage)
    #[arg(
        long,
        default_value_t = 80,
        requires = "coverage",
        value_parser = clap::value_parser!(u8).range(0..=100)
    )]
    pub min_coverage: u8,

    /// Run 'cargo check' (disabled by default in favor of Clippy)
    #[arg(long)]
    pub check: bool,
}

const fn cli_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .placeholder(AnsiColor::Blue.on_default())
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .valid(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
}
