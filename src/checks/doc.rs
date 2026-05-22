// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `cargo doc` with `RUSTDOCFLAGS=-D warnings` to fail on broken
//! intra-doc links and unresolvable references.

use super::{Check, Runner, cargo_outcome_with_env, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

const DOC_ARGS: &[&str] = &["--no-deps", "--workspace", "--all-features"];

pub struct DocCheck;

impl Check for DocCheck {
    fn label(&self) -> &'static str {
        "doc"
    }

    fn cmd(&self) -> String {
        format!(
            "RUSTDOCFLAGS='-D warnings' {}",
            fmt_cargo_cmd("doc", DOC_ARGS)
        )
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome_with_env(runner, "doc", DOC_ARGS, &[("RUSTDOCFLAGS", "-D warnings")])
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::DOC)
    }
}
