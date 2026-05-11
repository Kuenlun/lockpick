// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Unused-dependency detector. Wraps `cargo machete` which exits 0 on
//! a clean workspace and non-zero when unused deps are detected.

use super::{Check, run_cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct MacheteCheck;

impl Check for MacheteCheck {
    fn label(&self) -> &'static str {
        "machete"
    }

    fn cmd(&self) -> String {
        "cargo machete".to_string()
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("machete", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_machete() {
        assert_eq!(MacheteCheck.label(), "machete");
    }

    #[test]
    fn cmd_is_cargo_machete() {
        assert_eq!(MacheteCheck.cmd(), "cargo machete");
    }
}
