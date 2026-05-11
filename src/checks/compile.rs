// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, fmt_cargo_cmd, run_cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct CompileCheck;

impl Check for CompileCheck {
    fn label(&self) -> &'static str {
        "check"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("check", COMMON_ARGS)
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("check", COMMON_ARGS)
    }
}
