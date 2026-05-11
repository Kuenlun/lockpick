// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Unused-dependency detector. Wraps `cargo machete` which exits 0 on
//! a clean workspace and non-zero when unused deps are detected.

use super::{Check, Runner, cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct MacheteCheck;

impl Check for MacheteCheck {
    fn label(&self) -> &'static str {
        "machete"
    }

    fn cmd(&self) -> String {
        "cargo machete".to_string()
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "machete", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_machete() {
        assert_eq!(MacheteCheck.label(), "machete");
    }

    #[test]
    fn cmd_is_cargo_machete() {
        assert_eq!(MacheteCheck.cmd(), "cargo machete");
    }

    #[test]
    fn run_invokes_cargo_machete_with_no_extra_args() {
        let fake = FakeRunner::passing();
        assert!(MacheteCheck.run(&fake).passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "machete");
        assert!(calls[0].args.is_empty());
    }
}
