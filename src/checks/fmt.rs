// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;
use crate::tooling::ColorMode;

// `--all` is required: without it cargo silently formats only the
// root package. `-- --color <mode>` is the only way to silence
// rustfmt's diff colorizer, which ignores both `CARGO_TERM_COLOR`
// and `NO_COLOR`. Without it ANSI escapes leak into captured output.
const FMT_ARGS_ALWAYS: &[&str] = &["--all", "--check", "--", "--color", "always"];
const FMT_ARGS_NEVER: &[&str] = &["--all", "--check", "--", "--color", "never"];

pub struct FmtCheck {
    pub color: ColorMode,
}

impl FmtCheck {
    const fn args(&self) -> &'static [&'static str] {
        match self.color {
            ColorMode::Always => FMT_ARGS_ALWAYS,
            ColorMode::Never => FMT_ARGS_NEVER,
        }
    }
}

impl Check for FmtCheck {
    fn label(&self) -> &'static str {
        "fmt"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("fmt", self.args())
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "fmt", self.args())
    }

    fn chain_position(&self) -> Option<u8> {
        None
    }
}
