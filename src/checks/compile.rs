// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub struct CompileCheck;

impl CompileCheck {
    pub const LABEL: &'static str = "check";
}

impl Check for CompileCheck {
    fn label(&self) -> &'static str {
        Self::LABEL
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("check", COMMON_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "check", COMMON_ARGS)
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::COMPILE)
    }
}
