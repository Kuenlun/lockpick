// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Catalog of individual checks. Each module implements the [`Check`] trait
//! over its own struct so the runner stays decoupled from the specifics of
//! each cargo invocation.

use std::process::{Command, ExitStatus, Stdio};

use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::error::LockpickError;
use crate::reporter::{CheckOutcome, TaskStatus};
use crate::tooling;

pub mod audit;
pub mod clippy;
pub mod compile;
pub mod coverage;
pub mod doc;
pub mod doctest;
pub mod fmt;
pub mod license_header;
pub mod machete;
pub mod test;

pub const COMMON_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];

/// A single quality check that lockpick can execute.
pub trait Check: Send + Sync {
    /// Label shown in the spinner and in section headers.
    fn label(&self) -> &'static str;
    /// Human-readable command line for `--verbose` output.
    fn cmd(&self) -> String;
    /// Execute the check and capture its outcome.
    fn run(&self) -> CheckOutcome;
}

/// Build the list of parallel checks to run after the `compile` gate.
/// Skipped checks are excluded entirely so they don't appear in the output.
///
/// `coverage_active` enables instrumentation in the `test` check so its
/// `.profraw` files can be consumed by the coverage gate in phase 3.
#[must_use]
pub fn build_parallel(cli: &Cli, coverage_active: bool, config: &Config) -> Vec<Box<dyn Check>> {
    let mut checks: Vec<Box<dyn Check>> = Vec::new();

    if !cli.skips(&SkipOption::Clippy) {
        checks.push(Box::new(clippy::ClippyCheck));
    }
    if !cli.skips(&SkipOption::Fmt) {
        checks.push(Box::new(fmt::FmtCheck));
    }
    if !cli.skips(&SkipOption::Test) {
        checks.push(Box::new(test::TestCheck {
            instrumented: coverage_active,
            nextest: tooling::has_nextest(),
        }));
    }
    if !cli.skips(&SkipOption::DocTest) && doctest::workspace_has_lib_target() {
        checks.push(Box::new(doctest::DocTestCheck));
    }
    if !cli.skips(&SkipOption::Doc) {
        checks.push(Box::new(doc::DocCheck));
    }
    if !cli.skips(&SkipOption::Machete) {
        checks.push(Box::new(machete::MacheteCheck));
    }
    if !cli.skips(&SkipOption::Audit) {
        checks.push(Box::new(audit::AuditCheck));
    }
    if !cli.skips(&SkipOption::License)
        && let Some(header_path) = config.license_header.clone()
    {
        let globs = config
            .license_header_globs
            .clone()
            .unwrap_or_else(license_header::default_globs);
        checks.push(Box::new(license_header::LicenseHeaderCheck {
            header_path,
            globs,
        }));
    }

    checks
}

/// Shared executor. Runs `cargo <subcommand> <args…>` with optional
/// extra environment variables, captures stdout and stderr combined,
/// and returns the raw [`ExitStatus`] together with the captured output.
/// Redirects to `target/lockpick` when invoked from inside the project's
/// own `target/` directory to avoid self-locking on Windows.
pub fn run_cargo_with_env(
    subcommand: &str,
    args: &[&str],
    extra_envs: &[(&str, &str)],
) -> Result<(ExitStatus, String), LockpickError> {
    let mut cmd = Command::new("cargo");
    cmd.arg(subcommand).args(args);
    for (k, v) in extra_envs {
        cmd.env(k, v);
    }

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
    run_cargo_outcome_with_env(subcommand, args, &[])
}

/// Variant of [`run_cargo_outcome`] that injects extra environment variables.
pub fn run_cargo_outcome_with_env(
    subcommand: &str,
    args: &[&str],
    extra_envs: &[(&str, &str)],
) -> CheckOutcome {
    match run_cargo_with_env(subcommand, args, extra_envs) {
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

/// Helper to format a cargo command line for display.
pub fn fmt_cargo_cmd(subcommand: &str, args: &[&str]) -> String {
    if args.is_empty() {
        format!("cargo {subcommand}")
    } else {
        format!("cargo {subcommand} {}", args.join(" "))
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
