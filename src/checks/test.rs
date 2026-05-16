// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

// `--no-tests=pass`: align nextest >= 0.9.85 with `cargo test`'s default
// of treating zero discovered tests as success.
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
    /// Run tests through `cargo llvm-cov` to emit `.profraw` files.
    pub instrumented: bool,
    /// Prefer `cargo nextest` as the runner.
    pub nextest: bool,
}

impl TestCheck {
    pub const LABEL: &'static str = "test";

    const fn dispatch(&self) -> (&'static str, &'static [&'static str]) {
        match (self.instrumented, self.nextest) {
            (true, true) => ("llvm-cov", LLVM_COV_NEXTEST_ARGS),
            (true, false) => ("llvm-cov", LLVM_COV_ARGS),
            (false, true) => ("nextest", NEXTEST_PLAIN_ARGS),
            (false, false) => ("test", COMMON_ARGS),
        }
    }
}

impl Check for TestCheck {
    fn label(&self) -> &'static str {
        Self::LABEL
    }

    fn cmd(&self) -> String {
        let (sub, args) = self.dispatch();
        fmt_cargo_cmd(sub, args)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        let (sub, args) = self.dispatch();
        cargo_outcome(runner, sub, args)
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::TEST)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn label_constant_matches_the_value_the_orchestrator_relies_on() {
        assert_eq!(TestCheck::LABEL, "test");
        let c = TestCheck {
            instrumented: false,
            nextest: false,
        };
        assert_eq!(c.label(), TestCheck::LABEL);
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
    fn chain_position_is_test_for_every_runner_variant() {
        for (instrumented, nextest) in [(false, false), (false, true), (true, false), (true, true)]
        {
            let c = TestCheck {
                instrumented,
                nextest,
            };
            assert_eq!(
                c.chain_position(),
                Some(chain::TEST),
                "expected chain TEST slot for instrumented={instrumented} nextest={nextest}"
            );
        }
    }
}
