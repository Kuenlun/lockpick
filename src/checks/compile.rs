// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub struct CompileCheck;

impl Check for CompileCheck {
    fn label(&self) -> &'static str {
        "check"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("check", COMMON_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "check", COMMON_ARGS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_check() {
        assert_eq!(CompileCheck.label(), "check");
    }

    #[test]
    fn cmd_targets_workspace_with_all_targets_and_features() {
        let cmd = CompileCheck.cmd();
        assert!(cmd.starts_with("cargo check "));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-targets"));
        assert!(cmd.contains("--all-features"));
    }

    #[test]
    fn run_invokes_cargo_check_with_common_args() {
        let fake = FakeRunner::passing();
        assert!(CompileCheck.run(&fake).passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "check");
        assert!(calls[0].args.contains(&"--all-features".to_string()));
    }
}
