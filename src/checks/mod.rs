// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Individual checks. Each module implements [`Check`] over its own
//! struct, keeping the runner agnostic of the cargo invocation details.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::reporter::{CheckOutcome, TaskStatus};
use crate::tooling::{ColorMode, Tool, Toolchain, cargo_command};

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

/// Captured output of a finished cargo invocation. Synthesizable from
/// fakes, since [`std::process::ExitStatus`] has no public constructor.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Strategy that runs `cargo <sub> <args…>`. Production uses [`CargoCli`].
pub trait Runner: Send + Sync {
    /// Spawn the subcommand and capture its raw output.
    ///
    /// [`Err`] signals an OS-level launch failure; non-zero exits come
    /// back as `Ok(SpawnResult { success: false, … })`.
    fn spawn(
        &self,
        sub: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> std::io::Result<SpawnResult>;
}

/// Production [`Runner`]: shells out to the host `cargo`, scrubs
/// package-scoped env vars, and optionally redirects child builds away
/// from the parent's target directory.
#[derive(Debug, Clone, Copy, Default)]
pub struct CargoCli {
    /// When true, children inherit `CARGO_TARGET_DIR=target/lockpick`.
    redirect_target_dir: bool,
    /// Color decision propagated to every child as `CARGO_TERM_COLOR`,
    /// so captured output matches what lockpick will print on its own
    /// stdout stream.
    color: ColorMode,
}

impl CargoCli {
    /// Decide whether children need `CARGO_TARGET_DIR` redirected, and
    /// pin the color mode that will be propagated as `CARGO_TERM_COLOR`.
    #[must_use]
    pub fn detect(color: ColorMode) -> Self {
        Self {
            redirect_target_dir: needs_target_dir_redirect(
                std::env::current_exe().ok().as_deref(),
                std::env::current_dir().ok().as_deref(),
                std::env::var_os("CARGO_TARGET_DIR").as_deref(),
            ),
            color,
        }
    }
}

impl Runner for CargoCli {
    fn spawn(
        &self,
        sub: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> std::io::Result<SpawnResult> {
        let mut cmd = cargo_command();
        cmd.arg(sub).args(args);
        // Set our color decision first so caller-supplied `envs` still
        // win if a check ever needs to override it for one invocation.
        cmd.env("CARGO_TERM_COLOR", self.color.as_str());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        if self.redirect_target_dir {
            cmd.env("CARGO_TARGET_DIR", "target/lockpick");
        }
        execute(cmd)
    }
}

/// Spawn the [`Command`], record its PID so the SIGINT/SIGTERM handler
/// can forward signals to it, and capture both streams. The guard
/// returned by [`crate::signals::State::register_child`] removes the
/// PID on every exit path, including the early return from `?` and
/// unwind through the explicit `drop` site.
///
/// The guard is dropped immediately after `wait_with_output` reaps the
/// child, so the window in which a recycled PID could receive a
/// forwarded signal is bounded to a few instructions. A fully race-free
/// fix would need `pidfd_send_signal` (Linux 5.3+) or its BSD equivalent
/// to address the process by handle instead of by PID.
fn execute(mut cmd: Command) -> std::io::Result<SpawnResult> {
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let guard = crate::signals::state().register_child(child.id());
    let output = child.wait_with_output();
    drop(guard);
    output.map(|out| SpawnResult {
        success: out.status.success(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// Slot for a check inside the serial chain that competes for
/// `target/.cargo-lock`. Lower values run first; gaps are allowed.
///
/// The chain models the dependency every cargo build subcommand has on
/// the per-`target/` exclusive lock — running two of these in parallel
/// would just block on the lock and noisily print `Blocking waiting for
/// file lock`. See the `## Scheduling` section of the README.
pub mod chain {
    pub const COMPILE: u8 = 0;
    pub const TEST: u8 = 1;
    pub const CLIPPY: u8 = 2;
    pub const DOC: u8 = 3;
    pub const DOCTEST: u8 = 4;
}

/// A single quality check.
pub trait Check: Send + Sync {
    /// Label shown in spinners and section headers.
    fn label(&self) -> &'static str;
    /// Human-readable command line for `--verbose` output.
    fn cmd(&self) -> String;
    /// Execute the check.
    fn run(&self, runner: &dyn Runner) -> CheckOutcome;
    /// Position of this check inside the serial chain that competes
    /// for `target/.cargo-lock`. `None` marks an independent check
    /// safe to run in parallel with everything else (it does not
    /// touch `target/`).
    ///
    /// Canonical positions live in [`chain`]; lower runs first.
    fn chain_position(&self) -> Option<u8>;
}

/// The full schedule of checks that survived CLI/config gating.
///
/// Items keep insertion order so the verbose section list and the
/// final summary stay stable run-to-run. The runner partitions them
/// into two cohorts that Cargo's per-`target/` lock actually allows
/// to overlap: an independent cohort and a serial chain.
pub struct Plan {
    items: Vec<Box<dyn Check>>,
}

impl Plan {
    /// Number of checks scheduled, across both cohorts.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the plan has zero checks to run.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate every check with its insertion index, for display.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &dyn Check)> {
        self.items.iter().enumerate().map(|(i, c)| (i, c.as_ref()))
    }

    /// Checks that do not touch `target/` and so run in parallel with
    /// each other and with the serial chain.
    pub fn independent(&self) -> impl Iterator<Item = (usize, &dyn Check)> {
        self.iter().filter(|(_, c)| c.chain_position().is_none())
    }

    /// Checks that compete for `target/.cargo-lock`, sorted by their
    /// declared chain position so the runner walks them in canonical
    /// order — `compile → test → clippy → doc → doc-test` — regardless
    /// of insertion order.
    pub fn serial_chain(&self) -> impl Iterator<Item = (usize, &dyn Check)> {
        let mut chain: Vec<(u8, usize, &dyn Check)> = self
            .iter()
            .filter_map(|(i, c)| c.chain_position().map(|p| (p, i, c)))
            .collect();
        chain.sort_by_key(|(p, _, _)| *p);
        chain.into_iter().map(|(_, i, c)| (i, c))
    }
}

/// Assemble the [`Plan`] of checks that survived CLI/config gating.
///
/// Insertion order doubles as display order — the verbose section list
/// and the final summary follow it. Execution order inside the serial
/// chain is decoupled and lives in [`Check::chain_position`].
///
/// `coverage_active` instruments the `test` check so its `.profraw`
/// files feed the coverage gate; `has_lib` gates the doc-test check;
/// `branch_coverage` (true on nightly) passes `--branch` to the
/// instrumented test run; `color` is forwarded to the fmt check, whose
/// rustfmt diff renderer is the only subprocess that ignores the
/// `CARGO_TERM_COLOR` env var.
#[must_use]
pub fn build_plan(
    cli: &Cli,
    coverage_active: bool,
    toolchain: &Toolchain,
    config: &Config,
    has_lib: bool,
    branch_coverage: bool,
    color: ColorMode,
) -> Plan {
    let mut items: Vec<Box<dyn Check>> = Vec::new();

    if !cli.skips(&SkipOption::Check) {
        items.push(Box::new(compile::CompileCheck));
    }
    if !cli.skips(&SkipOption::Clippy) {
        items.push(Box::new(clippy::ClippyCheck));
    }
    if !cli.skips(&SkipOption::Fmt) {
        items.push(Box::new(fmt::FmtCheck { color }));
    }
    if !cli.skips(&SkipOption::Test) {
        items.push(Box::new(test::TestCheck {
            instrumented: coverage_active,
            nextest: toolchain.has(Tool::Nextest),
            branch_coverage,
        }));
    }
    if !cli.skips(&SkipOption::Doc) {
        items.push(Box::new(doc::DocCheck));
    }
    if !cli.skips(&SkipOption::DocTest) && has_lib {
        items.push(Box::new(doctest::DocTestCheck));
    }
    if !cli.skips(&SkipOption::Machete) {
        items.push(Box::new(machete::MacheteCheck));
    }
    if !cli.skips(&SkipOption::Audit) {
        items.push(Box::new(audit::AuditCheck));
    }
    if !cli.skips(&SkipOption::License)
        && let Some(header_path) = config.license_header.clone()
    {
        let globs = config
            .license_header_globs
            .clone()
            .unwrap_or_else(license_header::default_globs);
        items.push(Box::new(license_header::LicenseHeaderCheck {
            header_path,
            globs,
        }));
    }

    Plan { items }
}

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
pub fn fmt_cargo_cmd(subcommand: &str, args: &[&str]) -> String {
    if args.is_empty() {
        format!("cargo {subcommand}")
    } else {
        format!("cargo {subcommand} {}", args.join(" "))
    }
}

/// Whether child cargo invocations should redirect their target dir.
///
/// Redirects only when the running binary lives under `cwd/target/` and
/// `CARGO_TARGET_DIR` is unset.
pub fn needs_target_dir_redirect(
    exe: Option<&Path>,
    cwd: Option<&Path>,
    target_dir_env: Option<&std::ffi::OsStr>,
) -> bool {
    let (Some(exe), Some(cwd)) = (exe, cwd) else {
        return false;
    };
    target_dir_env.is_none() && exe.starts_with(cwd.join("target"))
}
