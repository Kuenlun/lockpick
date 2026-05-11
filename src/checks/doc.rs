// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Documentation build with rustdoc warnings escalated to errors.
//! Catches broken intra-doc links, unresolvable references and missing
//! examples in code blocks.

use super::{Check, run_cargo_outcome_with_env};
use crate::reporter::CheckOutcome;

pub struct DocCheck;

impl Check for DocCheck {
    fn label(&self) -> &'static str {
        "doc"
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome_with_env(
            "doc",
            &["--no-deps", "--workspace", "--all-features"],
            &[("RUSTDOCFLAGS", "-D warnings")],
        )
    }
}
