// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `--fix` phase: auto-apply formatter, clippy and machete fixes before
//! the verify pipeline. Streams subprocess output live; aborts at the
//! first failing step so the user never sees the same lint twice.

use crate::checks::{COMMON_ARGS, CargoCli, clippy::CLIPPY_LINT_ARGS, fmt_cargo_cmd};
use crate::cli::{Cli, SkipOption};
use crate::reporter::Reporter;

/// Run every enabled fix step in order. Returns `Err(())` if a step
/// fails or its launch errors; the caller maps that to the pipeline's
/// generic failure exit so the user sees the subprocess output we
/// already streamed plus the banner this module prints.
pub fn apply(cli: &Cli, runner: &CargoCli, reporter: &Reporter) -> Result<(), ()> {
    let clippy_args = clippy_fix_args();
    let steps: [(SkipOption, &str, &[&str]); 3] = [
        (SkipOption::Clippy, "clippy", &clippy_args),
        (SkipOption::Machete, "machete", &["--fix"]),
        (SkipOption::Fmt, "fmt", &["--all"]),
    ];
    for (skip, sub, args) in steps {
        if cli.skips(skip) {
            continue;
        }
        run_step(runner, reporter, sub, args)?;
    }
    Ok(())
}

/// Drive one fix step: banner, inherited spawn, error-on-failure.
fn run_step(runner: &CargoCli, reporter: &Reporter, sub: &str, args: &[&str]) -> Result<(), ()> {
    reporter.command(&fmt_cargo_cmd(sub, args));
    match runner.spawn_inherited(sub, args) {
        Ok(true) => Ok(()),
        Ok(false) => {
            reporter.note(&format!("fix: cargo {sub} exited with non-zero status"));
            Err(())
        }
        Err(e) => {
            reporter.note(&format!("fix: failed to launch cargo {sub}: {e}"));
            Err(())
        }
    }
}

/// Build the clippy fix argv: `--fix`, workspace prefix, dirty/staged
/// overrides (so users with WIP changes are not blocked), then the
/// shared lint tail behind `--`.
fn clippy_fix_args() -> Vec<&'static str> {
    let mut v = Vec::with_capacity(COMMON_ARGS.len() + CLIPPY_LINT_ARGS.len() + 4);
    v.push("--fix");
    v.extend_from_slice(COMMON_ARGS);
    v.push("--allow-dirty");
    v.push("--allow-staged");
    v.push("--");
    v.extend_from_slice(CLIPPY_LINT_ARGS);
    v
}
