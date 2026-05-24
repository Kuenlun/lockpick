// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! The [`Check`] trait, the [`chain`] of serial slot positions, and the
//! [`Plan`] the runner walks. `build_plan` is the single place where
//! CLI/config gating turns into the concrete list of checks to execute.

use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::reporter::CheckOutcome;
use crate::tooling::{ColorMode, Tool, Toolchain};

use super::runner::Runner;
use super::{audit, clippy, compile, doc, doctest, fmt, license_header, machete, test};

/// Slot for a check inside the serial chain that competes for
/// `target/.cargo-lock`. Lower values run first; gaps are allowed.
///
/// The chain models the dependency every cargo build subcommand has on
/// the per-`target/` exclusive lock. Running two of these in parallel
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
    /// declared chain position so the runner walks them in the canonical
    /// `compile → test → clippy → doc → doc-test` order regardless of
    /// insertion order.
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
/// Insertion order doubles as display order. The verbose section list
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

    if !cli.skips(SkipOption::Check) {
        items.push(Box::new(compile::CompileCheck));
    }
    if !cli.skips(SkipOption::Clippy) {
        items.push(Box::new(clippy::ClippyCheck));
    }
    if !cli.skips(SkipOption::Fmt) {
        items.push(Box::new(fmt::FmtCheck { color }));
    }
    if !cli.skips(SkipOption::Test) {
        items.push(Box::new(test::TestCheck {
            instrumented: coverage_active,
            nextest: toolchain.has(Tool::Nextest),
            branch_coverage,
        }));
    }
    if !cli.skips(SkipOption::Doc) {
        items.push(Box::new(doc::DocCheck));
    }
    if !cli.skips(SkipOption::DocTest) && has_lib {
        items.push(Box::new(doctest::DocTestCheck));
    }
    if !cli.skips(SkipOption::Machete) {
        items.push(Box::new(machete::MacheteCheck));
    }
    if !cli.skips(SkipOption::Audit) {
        items.push(Box::new(audit::AuditCheck));
    }
    if !cli.skips(SkipOption::License)
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
