// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub struct ClippyCheck;

impl Check for ClippyCheck {
    fn label(&self) -> &'static str {
        "clippy"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("clippy", COMMON_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "clippy", COMMON_ARGS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_clippy() {
        assert_eq!(ClippyCheck.label(), "clippy");
    }

    #[test]
    fn cmd_runs_cargo_clippy_on_workspace() {
        let cmd = ClippyCheck.cmd();
        assert!(cmd.starts_with("cargo clippy "));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-targets"));
        assert!(cmd.contains("--all-features"));
    }

    #[test]
    fn run_invokes_cargo_clippy_with_common_args() {
        let fake = FakeRunner::passing();
        assert!(ClippyCheck.run(&fake).passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "clippy");
        assert!(calls[0].args.contains(&"--workspace".to_string()));
    }
}
