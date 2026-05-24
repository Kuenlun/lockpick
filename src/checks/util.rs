// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Cross-cutting helpers shared by every check implementation: argv
//! conventions, stream stitching, outcome lowering, and command display.

use crate::reporter::{CheckOutcome, TaskStatus};

use super::runner::{Runner, SpawnResult};

/// Workspace-wide argv prefix shared by build-flavored checks.
pub const COMMON_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];

/// Concatenate `stdout` and `stderr`, inserting a newline between them
/// when stdout does not already end with one.
#[must_use]
pub fn combine_streams(stdout: &[u8], stderr: &[u8]) -> String {
    let mut combined = String::from_utf8_lossy(stdout).into_owned();
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&String::from_utf8_lossy(stderr));
    combined
}

/// Lower a [`Runner::spawn`] result into a [`CheckOutcome`]. A launch
/// failure becomes [`TaskStatus::Fail`] with empty output.
pub fn outcome_from(result: std::io::Result<SpawnResult>) -> CheckOutcome {
    match result {
        Ok(sr) => CheckOutcome {
            status: if sr.success {
                TaskStatus::Pass
            } else {
                TaskStatus::Fail
            },
            output: combine_streams(&sr.stdout, &sr.stderr),
        },
        Err(_) => CheckOutcome {
            status: TaskStatus::Fail,
            output: String::new(),
        },
    }
}

/// Spawn `cargo <sub> <args…>` and lower the result into a [`CheckOutcome`].
pub fn cargo_outcome(runner: &dyn Runner, sub: &str, args: &[&str]) -> CheckOutcome {
    outcome_from(runner.spawn(sub, args, &[]))
}

/// Like [`cargo_outcome`] but with extra env vars.
pub fn cargo_outcome_with_env(
    runner: &dyn Runner,
    sub: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> CheckOutcome {
    outcome_from(runner.spawn(sub, args, envs))
}

/// Format a cargo command line for display.
#[must_use]
pub fn fmt_cargo_cmd(subcommand: &str, args: &[&str]) -> String {
    if args.is_empty() {
        format!("cargo {subcommand}")
    } else {
        format!("cargo {subcommand} {}", args.join(" "))
    }
}
