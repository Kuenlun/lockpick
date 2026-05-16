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
const LLVM_COV_BRANCH_ARGS: &[&str] = &[
    "--branch",
    "--no-report",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-fail-fast",
];
const LLVM_COV_PLAIN_ARGS: &[&str] = &[
    "--no-report",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-fail-fast",
];
const LLVM_COV_NEXTEST_BRANCH_ARGS: &[&str] = &[
    "nextest",
    "--branch",
    "--no-report",
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-fail-fast",
    "--no-tests=pass",
];
const LLVM_COV_NEXTEST_PLAIN_ARGS: &[&str] = &[
    "nextest",
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
    /// Whether to pass `--branch` to `cargo llvm-cov`. Off on stable
    /// because `-Z coverage-options=branch` is nightly-only. Ignored
    /// when `instrumented` is false (plain `test`/`nextest` never see
    /// the flag).
    pub branch_coverage: bool,
}

impl TestCheck {
    pub const LABEL: &'static str = "test";

    const fn dispatch(&self) -> (&'static str, &'static [&'static str]) {
        match (self.instrumented, self.nextest, self.branch_coverage) {
            (true, true, true) => ("llvm-cov", LLVM_COV_NEXTEST_BRANCH_ARGS),
            (true, true, false) => ("llvm-cov", LLVM_COV_NEXTEST_PLAIN_ARGS),
            (true, false, true) => ("llvm-cov", LLVM_COV_BRANCH_ARGS),
            (true, false, false) => ("llvm-cov", LLVM_COV_PLAIN_ARGS),
            (false, true, _) => ("nextest", NEXTEST_PLAIN_ARGS),
            (false, false, _) => ("test", COMMON_ARGS),
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
            branch_coverage: false,
        };
        assert_eq!(c.label(), TestCheck::LABEL);
    }

    #[test]
    fn dispatch_plain_uses_cargo_test() {
        let c = TestCheck {
            instrumented: false,
            nextest: false,
            branch_coverage: false,
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
            branch_coverage: false,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "nextest");
        assert_eq!(args[0], "run");
        assert!(c.cmd().starts_with("cargo nextest run"));
    }

    #[test]
    fn dispatch_instrumented_with_branch_coverage_emits_branch_flag() {
        let c = TestCheck {
            instrumented: true,
            nextest: false,
            branch_coverage: true,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "llvm-cov");
        assert!(args.contains(&"--branch"));
        assert!(args.contains(&"--no-report"));
        assert!(c.cmd().contains("--no-fail-fast"));
    }

    #[test]
    fn dispatch_instrumented_without_branch_coverage_drops_branch_flag() {
        let c = TestCheck {
            instrumented: true,
            nextest: false,
            branch_coverage: false,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "llvm-cov");
        // `--branch` requires `-Z coverage-options=branch` (nightly), so
        // stable runs must invoke `cargo llvm-cov` without it. The rest
        // of the args (the bulk of the invocation) stay identical.
        assert!(!args.contains(&"--branch"));
        assert!(args.contains(&"--no-report"));
        assert!(!c.cmd().contains("--branch"));
    }

    #[test]
    fn dispatch_instrumented_with_nextest_uses_llvm_cov_nextest() {
        let c = TestCheck {
            instrumented: true,
            nextest: true,
            branch_coverage: true,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "llvm-cov");
        assert_eq!(args[0], "nextest");
        assert!(args.contains(&"--branch"));
        assert!(c.cmd().contains("llvm-cov nextest"));
    }

    #[test]
    fn dispatch_instrumented_nextest_without_branch_coverage_drops_branch_flag() {
        let c = TestCheck {
            instrumented: true,
            nextest: true,
            branch_coverage: false,
        };
        let (sub, args) = c.dispatch();
        assert_eq!(sub, "llvm-cov");
        assert_eq!(args[0], "nextest");
        assert!(!args.contains(&"--branch"));
        assert!(!c.cmd().contains("--branch"));
    }

    #[test]
    fn dispatch_plain_variants_ignore_branch_coverage_flag() {
        // `--branch` is only ever an `llvm-cov` flag; the plain runners
        // never see it regardless of how `branch_coverage` is set.
        for branch_coverage in [false, true] {
            for (instrumented, nextest) in [(false, false), (false, true)] {
                let c = TestCheck {
                    instrumented,
                    nextest,
                    branch_coverage,
                };
                let (_, args) = c.dispatch();
                assert!(
                    !args.contains(&"--branch"),
                    "plain variant must never carry --branch (instrumented={instrumented} nextest={nextest} branch_coverage={branch_coverage})",
                );
            }
        }
    }

    #[test]
    fn every_nextest_path_opts_out_of_no_tests_failure() {
        for (instrumented, nextest, branch_coverage) in [
            (false, true, false),
            (true, true, true),
            (true, true, false),
        ] {
            let c = TestCheck {
                instrumented,
                nextest,
                branch_coverage,
            };
            let (_, args) = c.dispatch();
            assert!(
                args.contains(&"--no-tests=pass"),
                "missing --no-tests=pass for instrumented={instrumented} nextest={nextest} branch_coverage={branch_coverage}"
            );
        }
    }

    #[test]
    fn chain_position_is_test_for_every_runner_variant() {
        for (instrumented, nextest, branch_coverage) in [
            (false, false, false),
            (false, true, false),
            (true, false, true),
            (true, false, false),
            (true, true, true),
            (true, true, false),
        ] {
            let c = TestCheck {
                instrumented,
                nextest,
                branch_coverage,
            };
            assert_eq!(
                c.chain_position(),
                Some(chain::TEST),
                "expected chain TEST slot for instrumented={instrumented} nextest={nextest} branch_coverage={branch_coverage}"
            );
        }
    }
}
