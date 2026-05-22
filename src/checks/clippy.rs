// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

// Strict policy enforced on every consumer crate: enable the three
// opt-in groups (pedantic, nursery, cargo) and escalate every warning
// to an error. `restriction` is excluded — its lints contradict each
// other and are meant to be picked à-la-carte.
const CLIPPY_ARGS: &[&str] = &[
    "--workspace",
    "--all-targets",
    "--all-features",
    "--",
    "-W",
    "clippy::pedantic",
    "-W",
    "clippy::nursery",
    "-W",
    "clippy::cargo",
    "-D",
    "warnings",
];

pub struct ClippyCheck;

impl Check for ClippyCheck {
    fn label(&self) -> &'static str {
        "clippy"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("clippy", CLIPPY_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "clippy", CLIPPY_ARGS)
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::CLIPPY)
    }
}
