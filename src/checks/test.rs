// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, COV_TEST_ARGS, Check, run_cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct TestCheck {
    /// When `true`, run tests through `cargo llvm-cov` so the resulting
    /// `.profraw` files can be consumed by the coverage gate.
    pub instrumented: bool,
}

impl Check for TestCheck {
    fn label(&self) -> &'static str {
        "test"
    }

    fn run(&self) -> CheckOutcome {
        if self.instrumented {
            run_cargo_outcome("llvm-cov", COV_TEST_ARGS)
        } else {
            run_cargo_outcome("test", COMMON_ARGS)
        }
    }
}
