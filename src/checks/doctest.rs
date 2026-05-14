// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Doc-test runner. Skipped on workspaces with no `lib` target.

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

const DOCTEST_ARGS: &[&str] = &["--doc", "--workspace", "--all-features"];

pub struct DocTestCheck;

impl Check for DocTestCheck {
    fn label(&self) -> &'static str {
        "doc test"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("test", DOCTEST_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "test", DOCTEST_ARGS)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn cmd_runs_cargo_test_doc() {
        let cmd = DocTestCheck.cmd();
        assert!(cmd.starts_with("cargo test "));
        assert!(cmd.contains("--doc"));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-features"));
    }
}
