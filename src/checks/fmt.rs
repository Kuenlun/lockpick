// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;
use crate::tooling::ColorMode;

// `--all` is required: without it cargo silently formats only the root
// package and skips workspace members. `-- --color <mode>` is the only
// way to silence rustfmt's diff colorizer: rustfmt's diff renderer
// ignores both `CARGO_TERM_COLOR` and `NO_COLOR`, so without this flag
// ANSI escapes leak into lockpick's captured output even when stdout is
// a pipe.
const FMT_ARGS_ALWAYS: &[&str] = &["--all", "--check", "--", "--color", "always"];
const FMT_ARGS_NEVER: &[&str] = &["--all", "--check", "--", "--color", "never"];

pub struct FmtCheck {
    pub color: ColorMode,
}

impl FmtCheck {
    const fn args(&self) -> &'static [&'static str] {
        match self.color {
            ColorMode::Always => FMT_ARGS_ALWAYS,
            ColorMode::Never => FMT_ARGS_NEVER,
        }
    }
}

impl Check for FmtCheck {
    fn label(&self) -> &'static str {
        "fmt"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("fmt", self.args())
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "fmt", self.args())
    }

    fn chain_position(&self) -> Option<u8> {
        None
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn cmd_runs_cargo_fmt_all_check_with_never_color_by_default() {
        assert_eq!(
            FmtCheck {
                color: ColorMode::Never,
            }
            .cmd(),
            "cargo fmt --all --check -- --color never",
        );
    }

    #[test]
    fn cmd_forwards_always_color_when_stdout_is_an_interactive_tty() {
        assert_eq!(
            FmtCheck {
                color: ColorMode::Always,
            }
            .cmd(),
            "cargo fmt --all --check -- --color always",
        );
    }

    #[test]
    fn chain_position_is_none_because_rustfmt_only_reads_sources() {
        assert_eq!(
            FmtCheck {
                color: ColorMode::Never,
            }
            .chain_position(),
            None,
        );
    }
}
