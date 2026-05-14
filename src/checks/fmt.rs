// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

// `--all` is required: without it cargo silently formats only the root
// package and skips workspace members.
const FMT_ARGS: &[&str] = &["--all", "--check"];

pub struct FmtCheck;

impl Check for FmtCheck {
    fn label(&self) -> &'static str {
        "fmt"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("fmt", FMT_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "fmt", FMT_ARGS)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn cmd_runs_cargo_fmt_all_check() {
        assert_eq!(FmtCheck.cmd(), "cargo fmt --all --check");
    }
}
