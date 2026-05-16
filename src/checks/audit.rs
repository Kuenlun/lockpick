// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `RustSec` advisory scan via `cargo audit`. Requires network access.

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub struct AuditCheck;

impl Check for AuditCheck {
    fn label(&self) -> &'static str {
        "audit"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("audit", &[])
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "audit", &[])
    }

    fn chain_position(&self) -> Option<u8> {
        None
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn cmd_is_cargo_audit() {
        assert_eq!(AuditCheck.cmd(), "cargo audit");
    }

    #[test]
    fn chain_position_is_none_because_audit_only_reads_the_lockfile() {
        assert_eq!(AuditCheck.chain_position(), None);
    }
}
