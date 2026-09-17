// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
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

pub(crate) struct TestCheck {
    pub(crate) options: super::util::BuildOptions,
    /// Run tests through `cargo llvm-cov` to emit `.profraw` files.
    pub(crate) instrumented: bool,
    /// Prefer `cargo nextest` as the runner.
    pub(crate) nextest: bool,
    /// Whether to pass `--branch` to `cargo llvm-cov`. Off on stable
    /// because `-Z coverage-options=branch` is nightly-only. Ignored
    /// when `instrumented` is false (plain `test`/`nextest` never see
    /// the flag).
    pub(crate) branch_coverage: bool,
}

impl TestCheck {
    const fn cleanup_args(&self) -> &'static [&'static str] {
        if self.options.locked {
            &["clean", "--workspace", "--locked"]
        } else {
            &["clean", "--workspace"]
        }
    }

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
        "test"
    }

    fn cmd(&self) -> String {
        let (sub, args) = self.dispatch();
        let command = fmt_cargo_cmd(sub, &self.options.args(args));
        if self.instrumented {
            format!(
                "{} && {command}",
                fmt_cargo_cmd("llvm-cov", self.cleanup_args())
            )
        } else {
            command
        }
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        let (sub, args) = self.dispatch();
        if self.instrumented {
            let cleanup = cargo_outcome(runner, "llvm-cov", self.cleanup_args());
            if !cleanup.passed() {
                return cleanup;
            }
        }
        cargo_outcome(runner, sub, &self.options.args(args))
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
    fn dispatch_routes_instrumentation_runner_and_branch_flag() {
        let cases = [
            (
                (true, true, true),
                ("llvm-cov", LLVM_COV_NEXTEST_BRANCH_ARGS),
            ),
            (
                (true, true, false),
                ("llvm-cov", LLVM_COV_NEXTEST_PLAIN_ARGS),
            ),
            ((true, false, true), ("llvm-cov", LLVM_COV_BRANCH_ARGS)),
            ((true, false, false), ("llvm-cov", LLVM_COV_PLAIN_ARGS)),
            ((false, true, false), ("nextest", NEXTEST_PLAIN_ARGS)),
            ((false, false, false), ("test", COMMON_ARGS)),
        ];
        for ((instrumented, nextest, branch_coverage), expected) in cases {
            let check = TestCheck {
                options: super::super::util::BuildOptions::default(),
                instrumented,
                nextest,
                branch_coverage,
            };
            assert_eq!(
                check.dispatch(),
                expected,
                "instrumented={instrumented} nextest={nextest} branch={branch_coverage}"
            );
        }
    }

    #[test]
    fn branch_flag_is_inert_without_instrumentation() {
        for nextest in [true, false] {
            let on = TestCheck {
                options: super::super::util::BuildOptions::default(),
                instrumented: false,
                nextest,
                branch_coverage: true,
            };
            let off = TestCheck {
                options: super::super::util::BuildOptions::default(),
                instrumented: false,
                nextest,
                branch_coverage: false,
            };
            assert_eq!(on.dispatch(), off.dispatch());
        }
    }

    #[test]
    fn cmd_renders_the_dispatched_argv() {
        let plain = TestCheck {
            options: super::super::util::BuildOptions::default(),
            instrumented: false,
            nextest: false,
            branch_coverage: false,
        };
        assert_eq!(
            plain.cmd(),
            "cargo test --workspace --all-targets --all-features"
        );
    }
    #[derive(Default)]
    struct SequenceRunner {
        calls: std::sync::Mutex<Vec<String>>,
        cleanup_succeeds: bool,
    }

    impl Runner for SequenceRunner {
        fn spawn(
            &self,
            sub: &str,
            args: &[&str],
            _envs: &[(&str, &str)],
        ) -> std::io::Result<super::super::runner::SpawnResult> {
            self.calls.lock().unwrap().push(fmt_cargo_cmd(sub, args));
            Ok(super::super::runner::SpawnResult {
                success: args.first() != Some(&"clean") || self.cleanup_succeeds,
                stdout: Vec::new(),
                stderr: b"cleanup output".to_vec(),
            })
        }
    }

    #[test]
    fn cleanup_precedes_instrumentation_and_failure_stops_tests() {
        let check = TestCheck {
            options: super::super::util::BuildOptions::default(),
            instrumented: true,
            nextest: false,
            branch_coverage: false,
        };
        for cleanup_succeeds in [true, false] {
            let runner = SequenceRunner {
                cleanup_succeeds,
                ..SequenceRunner::default()
            };
            let result = check.run(&runner);
            assert_eq!(result.passed(), cleanup_succeeds);
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls.first().unwrap(), "cargo llvm-cov clean --workspace");
            assert_eq!(calls.len(), if cleanup_succeeds { 2 } else { 1 });
        }
        let runner = SequenceRunner::default();
        let plain = TestCheck {
            instrumented: false,
            ..check
        };
        assert!(plain.run(&runner).passed());
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }
}
