// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

const TEST_PLAIN_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];
// `--no-tests=pass` matches `cargo test`'s behaviour when a target has zero
// test functions. Without it, nextest >= 0.9.85 exits non-zero by default,
// which makes lockpick's test gate flap based purely on whether nextest is
// installed on the host.
const NEXTEST_PLAIN_ARGS: &[&str] = &[
    "run",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-tests=pass",
];
const LLVM_COV_ARGS: &[&str] = &[
    "--branch",
    "--no-report",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-fail-fast",
];
const LLVM_COV_NEXTEST_ARGS: &[&str] = &[
    "nextest",
    "--branch",
    "--no-report",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-fail-fast",
    "--no-tests=pass",
];

pub struct TestCheck {
    /// When `true`, run tests through `cargo llvm-cov` so the resulting
    /// `.profraw` files can be consumed by the coverage gate.
    pub instrumented: bool,
    /// When `true`, prefer `cargo nextest` as the test runner.
    pub nextest: bool,
}

impl TestCheck {
    const fn dispatch(&self) -> (&'static str, &'static [&'static str]) {
        match (self.instrumented, self.nextest) {
            (true, true) => ("llvm-cov", LLVM_COV_NEXTEST_ARGS),
            (true, false) => ("llvm-cov", LLVM_COV_ARGS),
            (false, true) => ("nextest", NEXTEST_PLAIN_ARGS),
            (false, false) => ("test", TEST_PLAIN_ARGS),
        }
    }
}

impl Check for TestCheck {
    fn label(&self) -> &'static str {
        "test"
    }

    fn cmd(&self) -> String {
        let (sub, args) = self.dispatch();
        fmt_cargo_cmd(sub, args)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        let (sub, args) = self.dispatch();
        cargo_outcome(runner, sub, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_test() {
        let c = TestCheck {
            instrumented: false,
            nextest: false,
        };
        assert_eq!(c.label(), "test");
    }

    #[test]
    fn dispatch_plain_uses_cargo_test() {
        let c = TestCheck {
            instrumented: false,
            nextest: false,
        };
        let (sub, _) = c.dispatch();
        assert_eq!(sub, "test");
        assert!(c.cmd().starts_with("cargo test "));
    }

    #[test]
    fn dispatch_nextest_uses_cargo_nextest_run() {
        let c = TestCheck {
            instrumented: false,
            nextest: true,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "nextest");
        assert_eq!(args[0], "run");
        assert!(c.cmd().starts_with("cargo nextest run"));
    }

    #[test]
    fn dispatch_instrumented_uses_llvm_cov() {
        let c = TestCheck {
            instrumented: true,
            nextest: false,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "llvm-cov");
        assert!(args.contains(&"--branch"));
        assert!(args.contains(&"--no-report"));
        assert!(c.cmd().contains("--no-fail-fast"));
    }

    #[test]
    fn dispatch_instrumented_with_nextest_uses_llvm_cov_nextest() {
        let c = TestCheck {
            instrumented: true,
            nextest: true,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "llvm-cov");
        assert_eq!(args[0], "nextest");
        assert!(c.cmd().contains("llvm-cov nextest"));
    }

    /// Regression: nextest >= 0.9.85 defaults to exiting non-zero when the
    /// run discovers zero tests. Every nextest path must opt in to `pass`
    /// so lockpick's test gate stays consistent with `cargo test`.
    #[test]
    fn every_nextest_path_opts_out_of_no_tests_failure() {
        for (instrumented, nextest) in [(false, true), (true, true)] {
            let c = TestCheck {
                instrumented,
                nextest,
            };
            let (_, args) = c.dispatch();
            assert!(
                args.contains(&"--no-tests=pass"),
                "missing --no-tests=pass for instrumented={instrumented} nextest={nextest}"
            );
        }
    }

    #[test]
    fn run_forwards_dispatch_choice_to_runner() {
        for (instrumented, nextest, expected_sub) in [
            (false, false, "test"),
            (false, true, "nextest"),
            (true, false, "llvm-cov"),
            (true, true, "llvm-cov"),
        ] {
            let fake = FakeRunner::passing();
            let check = TestCheck {
                instrumented,
                nextest,
            };
            assert!(check.run(&fake).passed());
            let calls = fake.calls.lock().unwrap().clone();
            assert_eq!(
                calls[0].sub, expected_sub,
                "instrumented={instrumented} nextest={nextest}"
            );
        }
    }
}
