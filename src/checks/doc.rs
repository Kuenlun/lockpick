// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Documentation checks using the project's lint levels and rustdoc flags.

use super::{Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

const DOC_ARGS: &[&str] = &["--no-deps", "--workspace", "--all-features"];

pub(crate) struct DocCheck {
    pub(crate) options: super::util::BuildOptions,
}

impl Check for DocCheck {
    fn label(&self) -> &'static str {
        "doc"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("doc", &self.options.args(DOC_ARGS))
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "doc", &self.options.args(DOC_ARGS))
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::DOC)
    }
}
