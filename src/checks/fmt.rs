// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, fmt_cargo_cmd, run_cargo_outcome};
use crate::reporter::CheckOutcome;

const FMT_ARGS: &[&str] = &["--check"];

pub struct FmtCheck;

impl Check for FmtCheck {
    fn label(&self) -> &'static str {
        "fmt"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("fmt", FMT_ARGS)
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("fmt", FMT_ARGS)
    }
}
