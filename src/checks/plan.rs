// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
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
/// `target/.cargo-lock`. Lower values run first. Gaps are allowed.
///
/// Running two cargo build subcommands in parallel would block on
/// `target/.cargo-lock` and print `Blocking waiting for file lock`. See
/// `## How it schedules` in the README for the cohort layout.
pub(crate) mod chain {
    pub(crate) const COMPILE: u8 = 0;
    pub(crate) const TEST: u8 = 1;
    pub(crate) const CLIPPY: u8 = 2;
    pub(crate) const DOC: u8 = 3;
    pub(crate) const DOCTEST: u8 = 4;
}

/// A single quality check.
pub(crate) trait Check: Send + Sync {
    /// Label shown in spinners and section headers.
    fn label(&self) -> &'static str;
    /// Human-readable command line for `--verbose` output.
    fn cmd(&self) -> String;
    /// Execute the check.
    fn run(&self, runner: &dyn Runner) -> CheckOutcome;
    /// Slot inside the serial chain (lower runs first). `None` marks
    /// an independent check safe to run in parallel with everything
    /// else. Canonical positions live in [`chain`].
    fn chain_position(&self) -> Option<u8>;
}

/// The full schedule of checks that survived CLI/config gating.
/// Items keep insertion order for stable reporting. The runner
/// partitions them into an independent cohort and a serial chain that
/// Cargo's per-`target/` lock allows to overlap.
pub(crate) struct Plan {
    items: Vec<Box<dyn Check>>,
}

impl Plan {
    /// Number of checks scheduled, across both cohorts.
    #[must_use]
    pub(crate) const fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the plan has zero checks to run.
    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate every check with its insertion index, for display.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (usize, &dyn Check)> {
        self.items.iter().enumerate().map(|(i, c)| (i, c.as_ref()))
    }
}

/// Assemble the [`Plan`] of checks that survived CLI/config gating.
/// Insertion order is display order. Execution order inside the serial
/// chain lives in [`Check::chain_position`].
///
/// * `coverage_active` instruments `test` so its profraws feed coverage.
/// * `has_lib` gates the doc-test check.
/// * `branch_coverage` (true on nightly) passes `--branch` to the
///   instrumented test run.
/// * `color` is forwarded to the fmt check (rustfmt's diff ignores
///   `CARGO_TERM_COLOR`).
#[must_use]
pub(crate) fn build_plan(
    cli: &Cli,
    coverage_active: bool,
    toolchain: &Toolchain,
    config: &Config,
    has_lib: bool,
    branch_coverage: bool,
    color: ColorMode,
) -> Plan {
    let options = super::util::BuildOptions {
        locked: config.locked,
        host_target: config.host_target,
    };
    let mut items: Vec<Box<dyn Check>> = Vec::new();

    if !cli.skips(SkipOption::Check) {
        items.push(Box::new(compile::CompileCheck { options }));
    }
    if !cli.skips(SkipOption::Clippy) {
        items.push(Box::new(clippy::ClippyCheck { options }));
    }
    if !cli.skips(SkipOption::Fmt) {
        items.push(Box::new(fmt::FmtCheck { color }));
    }
    if !cli.skips(SkipOption::Test) {
        items.push(Box::new(test::TestCheck {
            options,
            instrumented: coverage_active,
            nextest: toolchain.has(Tool::Nextest),
            branch_coverage,
        }));
    }
    if !cli.skips(SkipOption::Doc) {
        items.push(Box::new(doc::DocCheck { options }));
    }
    if !cli.skips(SkipOption::DocTest) && has_lib {
        items.push(Box::new(doctest::DocTestCheck { options }));
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

    if !cli.skips(SkipOption::Targets) && !config.target_checks.is_empty() {
        let check = super::targets::TargetChecks::new(config, !cli.skips(SkipOption::Clippy));
        if !check.is_empty() {
            items.push(Box::new(check));
        }
    }
    Plan { items }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::checks::coverage::CoverageCheck;
    use crate::reporter::LABEL_WIDTH;

    fn plan_for(args: &[&str], config: &Config, has_lib: bool) -> Plan {
        let cli = Cli::parse_from(args.iter().copied());
        build_plan(
            &cli,
            false,
            &Toolchain::default(),
            config,
            has_lib,
            false,
            ColorMode::Never,
        )
    }

    fn labels(plan: &Plan) -> Vec<&'static str> {
        plan.iter().map(|(_, c)| c.label()).collect()
    }

    #[test]
    fn default_plan_lists_every_always_on_check_in_display_order() {
        let plan = plan_for(&["lockpick"], &Config::default(), true);
        assert_eq!(
            labels(&plan),
            [
                "check", "clippy", "fmt", "test", "doc", "doc-test", "machete", "audit"
            ]
        );
    }

    #[test]
    fn skip_flags_drop_their_checks() {
        let plan = plan_for(
            &["lockpick", "--skip", "check,clippy,test,doc,doc-test"],
            &Config::default(),
            true,
        );
        assert_eq!(labels(&plan), ["fmt", "machete", "audit"]);
    }

    #[test]
    fn doc_test_requires_a_lib_target() {
        let plan = plan_for(&["lockpick"], &Config::default(), false);
        assert!(!labels(&plan).contains(&"doc-test"));
    }

    #[test]
    fn license_check_joins_only_when_a_header_is_configured() {
        let config = Config {
            license_header: Some("hdr.txt".into()),
            ..Config::default()
        };
        assert!(labels(&plan_for(&["lockpick"], &config, true)).contains(&"license"));
        assert!(!labels(&plan_for(&["lockpick"], &Config::default(), true)).contains(&"license"));
    }

    #[test]
    fn skipping_everything_yields_an_empty_plan() {
        let plan = plan_for(
            &[
                "lockpick",
                "--skip",
                "check,clippy,test,doc-test,fmt,doc,machete,audit,license,coverage",
            ],
            &Config::default(),
            true,
        );
        assert!(plan.is_empty());
        assert_eq!(plan.len(), 0);
    }

    #[test]
    fn every_label_fits_the_reporter_column() {
        let config = Config {
            license_header: Some("hdr.txt".into()),
            ..Config::default()
        };
        let plan = plan_for(&["lockpick"], &config, true);
        for (_, check) in plan.iter() {
            assert!(
                check.label().len() <= LABEL_WIDTH,
                "label `{}` overflows the {LABEL_WIDTH}-column layout",
                check.label()
            );
        }
        assert!(CoverageCheck::LABEL.len() <= LABEL_WIDTH);
    }
}
