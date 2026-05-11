// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `RustSec` advisory check. Wraps `cargo audit`, which fetches the
//! advisory database (network required) and scans the workspace's
//! lockfile against known vulnerabilities.

use super::{Check, run_cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct AuditCheck;

impl Check for AuditCheck {
    fn label(&self) -> &'static str {
        "audit"
    }

    fn cmd(&self) -> String {
        "cargo audit".to_string()
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("audit", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_audit() {
        assert_eq!(AuditCheck.label(), "audit");
    }

    #[test]
    fn cmd_is_cargo_audit() {
        assert_eq!(AuditCheck.cmd(), "cargo audit");
    }
}
