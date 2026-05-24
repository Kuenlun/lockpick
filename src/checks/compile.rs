// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub struct CompileCheck;

impl Check for CompileCheck {
    fn label(&self) -> &'static str {
        "check"
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
