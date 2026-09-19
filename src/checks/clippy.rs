// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{COMMON_ARGS, Check, Runner, cargo_outcome, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

pub(crate) struct ClippyCheck {
    pub(crate) options: super::util::BuildOptions,
}

impl Check for ClippyCheck {
    fn label(&self) -> &'static str {
        "clippy"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("clippy", &self.options.args(COMMON_ARGS))
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "clippy", &self.options.args(COMMON_ARGS))
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::CLIPPY)
    }
}
