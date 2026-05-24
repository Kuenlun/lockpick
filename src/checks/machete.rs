// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Unused-dependency scan via `cargo machete`.

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub struct MacheteCheck;

impl Check for MacheteCheck {
    fn label(&self) -> &'static str {
        "machete"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("machete", &[])
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "machete", &[])
    }

    fn chain_position(&self) -> Option<u8> {
        None
    }
}
