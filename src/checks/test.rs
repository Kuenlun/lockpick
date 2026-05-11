// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, fmt_cargo_cmd, run_cargo_outcome};
use crate::reporter::CheckOutcome;

const TEST_PLAIN_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];
const NEXTEST_PLAIN_ARGS: &[&str] = &["run", "--workspace", "--all-targets", "--all-features"];
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

    fn run(&self) -> CheckOutcome {
        let (sub, args) = self.dispatch();
        run_cargo_outcome(sub, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
