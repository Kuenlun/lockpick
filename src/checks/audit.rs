// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `RustSec` advisory scan via `cargo audit`. Requires network access.

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::{CheckOutcome, TaskStatus};

pub struct AuditCheck;

impl Check for AuditCheck {
    fn label(&self) -> &'static str {
        "audit"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("audit", &[])
    }

    /// Convert a fetch-side failure (advisory DB unreachable) into
    /// `Skip` with a short reason, so a flaky GitHub or an offline CI
    /// box does not masquerade as a vulnerability finding. Real findings
    /// and other audit errors still propagate as `Fail`. Detection is
    /// substring-based because `cargo audit` exits `1` for both cases.
    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        let outcome = cargo_outcome(runner, "audit", &[]);
        if outcome.failed() && is_advisory_db_unreachable(&outcome.output) {
            return CheckOutcome {
                status: TaskStatus::Skip,
                output: "advisory database unreachable".to_string(),
            };
        }
        outcome
    }

    fn chain_position(&self) -> Option<u8> {
        None
    }
}

/// Lowercase substrings cargo-audit, libgit2, or the OS resolver print
/// when the advisory database cannot be fetched. Each marker must be
/// specific to an error path: `cargo audit` prints `Fetching advisory
/// database from …` on every run, so a generic substring would also
/// match successful runs that reported real vulnerabilities.
const UNREACHABLE_MARKERS: &[&str] = &[
    "couldn't fetch",
    "failed to fetch",
    "unable to access",
    "could not resolve",
    "network is unreachable",
    "connection refused",
    "connection timed out",
    "operation timed out",
    "temporary failure in name resolution",
];

fn is_advisory_db_unreachable(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    UNREACHABLE_MARKERS.iter().any(|m| lower.contains(m))
}
