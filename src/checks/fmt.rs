// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, run_cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct FmtCheck;

impl Check for FmtCheck {
    fn label(&self) -> &'static str {
        "fmt"
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("fmt", &["--check"])
    }
}
