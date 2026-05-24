// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

// Strict policy enforced on every consumer crate: enable the three
// opt-in groups (pedantic, nursery, cargo) and escalate every warning
// to an error. `restriction` is excluded: its lints contradict each
// other and are meant to be picked à-la-carte.
//
// Split from the workspace prefix so `--fix` can reuse the exact same
// lint tail without `--` getting in the way.
pub const CLIPPY_LINT_ARGS: &[&str] = &[
    "-W",
    "clippy::pedantic",
    "-W",
    "clippy::nursery",
    "-W",
    "clippy::cargo",
    "-D",
    "warnings",
];

/// Argv handed to `cargo clippy` for the check: workspace prefix, `--`,
/// then the shared lint tail. Materialised at compile time so `cmd()`
/// and `run()` see a stable `&'static [&'static str]`.
const CLIPPY_ARGS: &[&str] = &concat_clippy_args();

const fn concat_clippy_args() -> [&'static str; COMMON_ARGS.len() + 1 + CLIPPY_LINT_ARGS.len()] {
    let mut out = [""; COMMON_ARGS.len() + 1 + CLIPPY_LINT_ARGS.len()];
    let mut i = 0;
    while i < COMMON_ARGS.len() {
        out[i] = COMMON_ARGS[i];
        i += 1;
    }
    out[COMMON_ARGS.len()] = "--";
    let mut j = 0;
    while j < CLIPPY_LINT_ARGS.len() {
        out[COMMON_ARGS.len() + 1 + j] = CLIPPY_LINT_ARGS[j];
        j += 1;
    }
    out
}

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
