// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

// Strict policy: enable the three opt-in groups (pedantic, nursery,
// cargo) and escalate every warning to an error. `restriction` is
// excluded because its lints contradict each other by design.
// `multiple_crate_versions` is carved out of the cargo group: duplicate
// versions almost always come from transitive dependencies the project
// under check cannot fix, so failing on them punishes the wrong party.
//
// Split from the workspace prefix so `--fix` can reuse the exact same
// lint tail without `--` in the middle.
pub const CLIPPY_LINT_ARGS: &[&str] = &[
    "-W",
    "clippy::pedantic",
    "-W",
    "clippy::nursery",
    "-W",
    "clippy::cargo",
    "-A",
    "clippy::multiple_crate_versions",
    "-D",
    "warnings",
];

/// Argv for `cargo clippy`: workspace prefix, `--`, then the shared
/// lint tail. Materialised at compile time so `cmd()` and `run()` see
/// a stable `&'static [&'static str]`.
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn argv_is_workspace_prefix_then_separator_then_lint_tail() {
        let (prefix, rest) = CLIPPY_ARGS.split_at(COMMON_ARGS.len());
        assert_eq!(prefix, COMMON_ARGS);
        assert_eq!(rest[0], "--");
        assert_eq!(&rest[1..], CLIPPY_LINT_ARGS);
    }

    #[test]
    fn transitive_duplicate_versions_are_exempted() {
        // `-A` must come after `-W clippy::cargo` so it wins for that
        // single lint while the rest of the group stays escalated.
        let allow = CLIPPY_LINT_ARGS
            .iter()
            .position(|a| *a == "clippy::multiple_crate_versions")
            .expect("exemption missing from lint tail");
        assert_eq!(CLIPPY_LINT_ARGS[allow - 1], "-A");
        let cargo_group = CLIPPY_LINT_ARGS
            .iter()
            .position(|a| *a == "clippy::cargo")
            .expect("cargo group missing from lint tail");
        assert!(cargo_group < allow);
    }
}
