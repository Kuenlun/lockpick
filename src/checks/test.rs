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
