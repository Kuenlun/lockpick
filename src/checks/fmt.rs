// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

const FMT_ARGS: &[&str] = &["--check"];

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
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_fmt() {
        assert_eq!(FmtCheck.label(), "fmt");
    }

    #[test]
    fn cmd_runs_cargo_fmt_check() {
        assert_eq!(FmtCheck.cmd(), "cargo fmt --check");
    }

    #[test]
    fn run_invokes_cargo_fmt_check() {
        let fake = FakeRunner::passing();
        assert!(FmtCheck.run(&fake).passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "fmt");
        assert!(calls[0].args.contains(&"--check".to_string()));
    }
}
