// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Catalog of individual checks. Each module implements the [`Check`] trait
//! over its own struct so the runner stays decoupled from the specifics of
//! each cargo invocation.

use std::process::{Command, ExitStatus, Stdio};

use crate::cli::{Cli, SkipOption};
use crate::error::LockpickError;
use crate::reporter::{CheckOutcome, TaskStatus};

pub mod clippy;
pub mod compile;
pub mod doctest;
pub mod fmt;
pub mod test;

pub const COMMON_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];
pub const COV_TEST_ARGS: &[&str] = &[
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-fail-fast",
];

/// A single quality check that lockpick can execute.
pub trait Check: Send + Sync {
    /// Label shown in the spinner and in section headers.
    fn label(&self) -> &'static str;
    /// Execute the check and capture its outcome.
    fn run(&self) -> CheckOutcome;
}

/// Build the list of parallel checks to run after the `compile` gate.
/// Skipped checks are excluded entirely so they don't appear in the output.
#[must_use]
pub fn build_parallel(cli: &Cli, has_llvm_cov: bool) -> Vec<Box<dyn Check>> {
    let mut checks: Vec<Box<dyn Check>> = Vec::new();

    if !cli.skips(&SkipOption::Clippy) {
        checks.push(Box::new(clippy::ClippyCheck));
    }
    if !cli.skips(&SkipOption::Fmt) {
        checks.push(Box::new(fmt::FmtCheck));
    }
    if !cli.skips(&SkipOption::Test) {
        checks.push(Box::new(test::TestCheck {
            instrumented: cli.opt_in.coverage && has_llvm_cov,
        }));
    }
    if !cli.skips(&SkipOption::DocTest) && doctest::workspace_has_lib_target() {
        checks.push(Box::new(doctest::DocTestCheck));
    }

    checks
}

/// Shared executor. Runs `cargo <subcommand> <args…>`, captures both
/// stdout and stderr combined, and returns the raw [`ExitStatus`] together
/// with the captured output. Redirects to `target/lockpick` when invoked
/// from inside the project's own `target/` directory to avoid self-locking
/// on Windows.
pub fn run_cargo(subcommand: &str, args: &[&str]) -> Result<(ExitStatus, String), LockpickError> {
    log::info!("cargo {subcommand} {}", args.join(" "));

    let mut cmd = Command::new("cargo");
    cmd.arg(subcommand).args(args);

    if exe_in_target_dir() && std::env::var_os("CARGO_TARGET_DIR").is_none() {
        cmd.env("CARGO_TARGET_DIR", "target/lockpick");
    }

    let output = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut combined = stdout.into_owned();
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&stderr);

    Ok((output.status, combined))
}

/// Convenience wrapper that lowers an executor failure to a Fail outcome
/// with empty output, matching the original runner behavior.
pub fn run_cargo_outcome(subcommand: &str, args: &[&str]) -> CheckOutcome {
    match run_cargo(subcommand, args) {
        Ok((status, output)) => CheckOutcome {
            status: if status.success() {
                TaskStatus::Pass
            } else {
                TaskStatus::Fail
            },
            output,
        },
        Err(_) => CheckOutcome {
            status: TaskStatus::Fail,
            output: String::new(),
        },
    }
}

/// Returns `true` when the running binary lives inside the project's
/// `target/` directory (i.e. launched via `cargo run`). In that case
/// child cargo invocations would contend with the parent for the same
/// target directory, and on Windows the running `.exe` is locked by the
/// OS so a rebuild would fail with "Access denied".
fn exe_in_target_dir() -> bool {
    let (Ok(exe), Ok(cwd)) = (std::env::current_exe(), std::env::current_dir()) else {
        return false;
    };
    exe.starts_with(cwd.join("target"))
}
