// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub struct CompileCheck;

impl CompileCheck {
    pub const LABEL: &'static str = "check";
}

impl Check for CompileCheck {
    fn label(&self) -> &'static str {
        Self::LABEL
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("check", COMMON_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "check", COMMON_ARGS)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn cmd_targets_workspace_with_all_targets_and_features() {
        let cmd = CompileCheck.cmd();
        assert!(cmd.starts_with("cargo check "));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-targets"));
        assert!(cmd.contains("--all-features"));
    }

    #[test]
    fn label_constant_matches_trait_method() {
        assert_eq!(CompileCheck::LABEL, "check");
        assert_eq!(CompileCheck.label(), CompileCheck::LABEL);
    }
}
