// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `RustSec` advisory scan via `cargo audit`. Requires network access.

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub(crate) struct AuditCheck;

impl Check for AuditCheck {
    fn label(&self) -> &'static str {
        "audit"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("audit", &["--deny", "warnings"])
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "audit", &["--deny", "warnings"])
    }

    fn chain_position(&self) -> Option<u8> {
        None
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::checks::runner::SpawnResult;

    /// Stub [`Runner`] returning the same canned result for every spawn.
    struct CannedRunner {
        success: bool,
        stderr: &'static str,
    }

    impl Runner for CannedRunner {
        fn spawn(
            &self,
            _sub: &str,
            _args: &[&str],
            _envs: &[(&str, &str)],
        ) -> std::io::Result<SpawnResult> {
            Ok(SpawnResult {
                success: self.success,
                stdout: Vec::new(),
                stderr: self.stderr.as_bytes().to_vec(),
            })
        }
    }

    #[test]
    fn unreachable_advisory_db_fails_with_original_diagnostic() {
        let runner = CannedRunner {
            success: false,
            stderr: "error: couldn't fetch advisory database",
        };
        let outcome = AuditCheck.run(&runner);
        assert!(outcome.failed());
        assert!(outcome.output.contains("couldn't fetch advisory database"));
    }

    #[test]
    fn real_findings_stay_failures() {
        let runner = CannedRunner {
            success: false,
            stderr: "error: 1 vulnerability found!",
        };
        assert!(AuditCheck.run(&runner).failed());
    }

    #[test]
    fn clean_audit_passes() {
        let runner = CannedRunner {
            success: true,
            stderr: "Fetching advisory database from github",
        };
        assert!(AuditCheck.run(&runner).passed());
    }
}
