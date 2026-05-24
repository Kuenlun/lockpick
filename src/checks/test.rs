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
