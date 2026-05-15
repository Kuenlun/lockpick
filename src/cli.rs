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

impl SkipOption {
    /// Kebab-case identifier this variant accepts as `--skip <value>`.
    /// Single source of truth so hints in error messages cannot drift
    /// from what clap actually parses (locked by a test below).
    #[must_use]
    pub const fn skip_flag(&self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Clippy => "clippy",
            Self::Test => "test",
            Self::DocTest => "doc-test",
            Self::Fmt => "fmt",
            Self::Doc => "doc",
            Self::Machete => "machete",
            Self::Audit => "audit",
            Self::License => "license",
            Self::Coverage => "coverage",
        }
    }
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    /// Anchor `skip_flag` to clap's derived name for every variant, so a
    /// rename or kebab-case tweak fails here instead of silently shipping
    /// a hint pointing at a flag clap no longer accepts.
    #[test]
    fn skip_flag_matches_clap_value_enum_name_for_every_variant() {
        for variant in SkipOption::value_variants() {
            assert_eq!(
                variant.skip_flag(),
                variant.to_possible_value().unwrap().get_name(),
                "skip_flag drift for {variant:?}",
            );
        }
    }
}
