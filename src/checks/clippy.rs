// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, fmt_cargo_cmd, run_cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct ClippyCheck;

impl Check for ClippyCheck {
    fn label(&self) -> &'static str {
        "clippy"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("clippy", COMMON_ARGS)
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("clippy", COMMON_ARGS)
    }
}
