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
pub(crate) const CLIPPY_LINT_ARGS: &[&str] = &[
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

/// Share one argument constructor between execution and display.
fn clippy_args(options: super::util::BuildOptions) -> Vec<&'static str> {
    let mut args = options.args(COMMON_ARGS);
    args.push("--");
    args.extend_from_slice(CLIPPY_LINT_ARGS);
    args
}

pub(crate) struct ClippyCheck {
    pub(crate) options: super::util::BuildOptions,
}

impl Check for ClippyCheck {
    fn label(&self) -> &'static str {
        "clippy"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("clippy", &clippy_args(self.options))
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "clippy", &clippy_args(self.options))
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
        let args = clippy_args(super::super::util::BuildOptions::default());
        let (prefix, rest) = args.split_at(COMMON_ARGS.len());
        assert_eq!(prefix, COMMON_ARGS);
        assert_eq!(rest.first(), Some(&"--"));
        assert_eq!(
            rest.iter().skip(1).copied().collect::<Vec<_>>(),
            CLIPPY_LINT_ARGS
        );
    }

    #[test]
    fn transitive_duplicate_versions_are_exempted() {
        // `-A` must come after `-W clippy::cargo` so it wins for that
        // single lint while the rest of the group stays escalated.
        let allow = CLIPPY_LINT_ARGS
            .iter()
            .position(|a| *a == "clippy::multiple_crate_versions")
            .expect("exemption missing from lint tail");
        assert!(
            CLIPPY_LINT_ARGS
                .windows(2)
                .any(|pair| pair == ["-A", "clippy::multiple_crate_versions"])
        );
        let cargo_group = CLIPPY_LINT_ARGS
            .iter()
            .position(|a| *a == "clippy::cargo")
            .expect("cargo group missing from lint tail");
        assert!(cargo_group < allow);
    }
}
