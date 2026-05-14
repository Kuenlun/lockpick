// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use clap::{
    Parser, ValueEnum,
    builder::styling::{AnsiColor, Effects, Styles},
};

/// Check identifier for `--skip`.
#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum SkipOption {
    Check,
    Clippy,
    Test,
    DocTest,
    Fmt,
    Doc,
    Machete,
    Audit,
    License,
    Coverage,
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Rust merge-check CLI. Runs compile, lints, formatting, tests \
             and 100% coverage in a single invocation.",
    long_about = None,
    styles = cli_styles()
)]
pub struct Cli {
    /// Skip one or more checks (e.g. --skip clippy --skip fmt)
    #[arg(long, value_enum)]
    pub skip: Vec<SkipOption>,

    /// Show every command and the full output of all checks (CI mode)
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
}

impl Cli {
    #[must_use]
    pub fn skips(&self, option: &SkipOption) -> bool {
        self.skip.contains(option)
    }
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
