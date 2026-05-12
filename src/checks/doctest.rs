// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Doc-test runner. Only added to the parallel set when the workspace
//! actually exposes a `lib` target (see [`crate::config::LockpickMetadata`]).

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
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_doc_test() {
        assert_eq!(DocTestCheck.label(), "doc test");
    }

    #[test]
    fn cmd_runs_cargo_test_doc() {
        let cmd = DocTestCheck.cmd();
        assert!(cmd.starts_with("cargo test "));
        assert!(cmd.contains("--doc"));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-features"));
    }

    #[test]
    fn run_invokes_cargo_test_with_doc_args() {
        let fake = FakeRunner::passing();
        assert!(DocTestCheck.run(&fake).passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "test");
        assert!(calls[0].args.contains(&"--doc".to_string()));
    }
}
