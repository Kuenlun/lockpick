// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

// Strict policy enforced on every consumer crate: enable the three
// opt-in groups (pedantic, nursery, cargo) and escalate every warning
// to an error. `restriction` is excluded — its lints contradict each
// other and are meant to be picked à-la-carte.
const CLIPPY_ARGS: &[&str] = &[
    "--workspace",
    "--all-targets",
    "--all-features",
    "--",
    "-W",
    "clippy::pedantic",
    "-W",
    "clippy::nursery",
    "-W",
    "clippy::cargo",
    "-D",
    "warnings",
];

pub struct ClippyCheck;

impl Check for ClippyCheck {
    fn label(&self) -> &'static str {
        "clippy"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("clippy", CLIPPY_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "clippy", CLIPPY_ARGS)
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::CLIPPY)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn cmd_runs_cargo_clippy_on_workspace_with_strict_policy() {
        let cmd = ClippyCheck.cmd();
        assert!(cmd.starts_with("cargo clippy "));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-targets"));
        assert!(cmd.contains("--all-features"));
        assert!(cmd.contains("-W clippy::pedantic"));
        assert!(cmd.contains("-W clippy::nursery"));
        assert!(cmd.contains("-W clippy::cargo"));
        assert!(cmd.contains("-D warnings"));
    }

    #[test]
    fn chain_position_is_clippy_so_it_runs_after_test_in_the_chain() {
        assert_eq!(ClippyCheck.chain_position(), Some(chain::CLIPPY));
    }
}
