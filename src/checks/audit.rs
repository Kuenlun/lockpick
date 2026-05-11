// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `RustSec` advisory check. Wraps `cargo audit`, which fetches the
//! advisory database (network required) and scans the workspace's
//! lockfile against known vulnerabilities.

use super::{Check, Runner, cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct AuditCheck;

impl Check for AuditCheck {
    fn label(&self) -> &'static str {
        "audit"
    }

    fn cmd(&self) -> String {
        "cargo audit".to_string()
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "audit", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_audit() {
        assert_eq!(AuditCheck.label(), "audit");
    }

    #[test]
    fn cmd_is_cargo_audit() {
        assert_eq!(AuditCheck.cmd(), "cargo audit");
    }

    #[test]
    fn run_invokes_cargo_audit_with_no_extra_args() {
        let fake = FakeRunner::passing();
        let outcome = AuditCheck.run(&fake);
        assert!(outcome.passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].sub, "audit");
        assert!(calls[0].args.is_empty());
    }

    #[test]
    fn run_propagates_runner_failure() {
        let fake = FakeRunner::failing();
        assert!(AuditCheck.run(&fake).failed());
    }
}
