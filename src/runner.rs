// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::io::IsTerminal;
use std::thread;

use indicatif::ProgressBar;

use crate::checks::{self, CargoCli, Check, Plan, Runner, chain, coverage::CoverageCheck};
use crate::cli::{Cli, SkipOption};
use crate::config::{Config, LockpickMetadata};
use crate::error::{LockpickError, MissingTool};
use crate::fix;
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling::{self, ColorMode, Tool, Toolchain};

/// Run the full check pipeline. Loads tooling, config and workspace
/// metadata, then orchestrates the independent cohort, the serial chain
/// and coverage.
pub fn run(mut cli: Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::auto(cli.verbose);
    let toolchain = Toolchain::detect();
    let metadata = LockpickMetadata::load();
    // Fold any `skip = [...]` from Cargo.toml into the CLI's view of
    // skips so every downstream consumer reads from a single source.
    cli.merge_config_skips(&metadata.config.skip);
    // Color decision drives both the subprocess vocabulary
    // (`CARGO_TERM_COLOR`, rustfmt's `--color`) and the global `colored`
    // override, so the user's `--color`/`NO_COLOR`/TTY signals land
    // coherently across our own output and every cargo child.
    let color = cli.color_mode(std::io::stdout().is_terminal());
    // Process-wide override: every other crate linked into this process inherits it too.
    colored::control::set_override(color == ColorMode::Always);
    let runner = CargoCli::detect(color, metadata.workspace_root.clone());
    // Probe the toolchain once at startup. Both the early `branches`-on-
    // stable gate and the per-check `--branch` argv key off this single
    // boolean, so caching is wasted state.
    let is_nightly = tooling::is_nightly();
    let config = &metadata.config;
    let has_lib = metadata.has_lib_target;

    let coverage_active = is_coverage_active(&cli);

    require_tooling(&cli, coverage_active, &toolchain)?;
    require_nightly_for_branches(coverage_active, config, is_nightly)?;

    // Fix phase runs before the verify pipeline so the same invocation
    // can heal the tree and then prove it. Abort on failure: the
    // pipeline would only refail on the same lint with the same output.
    if cli.fix && fix::apply(&cli, &runner, &reporter).is_err() {
        return Err(LockpickError::ChecksFailed(1));
    }

    // Branch coverage measurement is gated on nightly because
    // `-Z coverage-options=branch` is unstable. Stable runs still get
    // functions/lines/regions; only the branches metric is dropped.
    let branch_coverage = is_nightly;

    let plan = checks::build_plan(
        &cli,
        coverage_active,
        &toolchain,
        config,
        has_lib,
        branch_coverage,
        color,
    );
    let coverage_check = coverage_active.then_some(CoverageCheck {
        thresholds: config.coverage,
        branch_coverage,
    });

    // Coverage rides on `test`, which is the only path that emits the
    // profraw files coverage consumes. If `test` did not survive the
    // CLI, coverage cannot have either. The assert pins that invariant
    // for future refactors.
    if plan.is_empty() {
        debug_assert!(
            coverage_check.is_none(),
            "invariant: empty `plan` must imply no coverage check"
        );
        return Err(LockpickError::NoChecksToRun);
    }

    if cli.skips(SkipOption::Test) && !cli.skips(SkipOption::Coverage) {
        reporter.note("--skip test implies coverage will be skipped");
    }
    if cli.skips(SkipOption::DocTest) && !has_lib {
        reporter.note("--skip doc-test has no effect: workspace has no lib target");
    }
    if cli.skips(SkipOption::License) && config.license_header.is_none() {
        reporter.note("--skip license has no effect: no license_header configured");
    }
    if coverage_active && !is_nightly {
        reporter.note("branch coverage disabled: requires nightly");
    }

    if reporter.is_verbose {
        print_planned_commands(
            &reporter,
            &plan,
            coverage_check.as_ref().map(|c| c as &dyn Check),
        );
    }

    let pbs: Vec<ProgressBar> = plan
        .iter()
        .map(|(_, c)| reporter.add_spinner(c.label()))
        .collect();
    let coverage_pb = coverage_check
        .as_ref()
        .map(|c| reporter.add_spinner(c.label()));
    let coverage = coverage_check.as_ref().zip(coverage_pb.as_ref());

    let (outcomes, coverage_outcome) = run_pipeline(&plan, &pbs, coverage, &reporter, &runner);

    let items = flatten_outcomes(&plan, &outcomes, coverage_outcome.as_ref());
    let failure_count = report_results(&reporter, &items);

    if failure_count > 0 {
        return Err(LockpickError::ChecksFailed(failure_count));
    }

    Ok(())
}

/// Whether the coverage gate runs. Disabled by `--skip coverage` or by
/// `--skip test` (no instrumentation, no coverage).
fn is_coverage_active(cli: &Cli) -> bool {
    !cli.skips(SkipOption::Coverage) && !cli.skips(SkipOption::Test)
}

/// Refuse to run when the user configured `coverage.branches` but the
/// active toolchain is stable. Branch coverage relies on
/// `-Z coverage-options=branch` (nightly-only), and degrading silently
/// to a non-branch measurement would mask the user's explicit threshold.
const fn require_nightly_for_branches(
    coverage_active: bool,
    config: &Config,
    is_nightly: bool,
) -> Result<(), LockpickError> {
    if coverage_active && config.coverage.branches.is_some() && !is_nightly {
        Err(LockpickError::BranchesRequireNightly)
    } else {
        Ok(())
    }
}

/// Collect every absent cargo subcommand at once so the user can install
/// all of them in a single `cargo install …` invocation instead of
/// re-running lockpick after each one.
fn require_tooling(
    cli: &Cli,
    coverage_active: bool,
    toolchain: &Toolchain,
) -> Result<(), LockpickError> {
    let mut missing = Vec::new();
    if coverage_active && !toolchain.has(Tool::LlvmCov) {
        missing.push(MissingTool {
            binary: "cargo-llvm-cov",
            skip_flag: SkipOption::Coverage.skip_flag(),
        });
    }
    if !cli.skips(SkipOption::Machete) && !toolchain.has(Tool::Machete) {
        missing.push(MissingTool {
            binary: "cargo-machete",
            skip_flag: SkipOption::Machete.skip_flag(),
        });
    }
    if !cli.skips(SkipOption::Audit) && !toolchain.has(Tool::Audit) {
        missing.push(MissingTool {
            binary: "cargo-audit",
            skip_flag: SkipOption::Audit.skip_flag(),
        });
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(LockpickError::MissingTools(missing))
    }
}

/// Render one banner line per planned cargo invocation, plus a trailing
/// blank line. Caller is responsible for the `is_verbose` gate.
fn print_planned_commands(reporter: &Reporter, plan: &Plan, coverage: Option<&dyn Check>) {
    for (_, c) in plan.iter() {
        reporter.command(&c.cmd());
    }
    if let Some(c) = coverage {
        reporter.command(&c.cmd());
    }
    reporter.diagln("");
}

/// Run a single check and finish its progress bar from the same thread.
/// PASS/FAIL marks land as soon as the check ends, regardless of which
/// cohort it belongs to.
fn run_one(
    check: &dyn Check,
    pb: &ProgressBar,
    reporter: &Reporter,
    runner: &dyn Runner,
) -> CheckOutcome {
    let outcome = check.run(runner);
    reporter.finish_spinner(pb, check.label(), outcome.status);
    // A check that downgrades itself to `Skip` carries a short reason
    // in `output`; surface it inline so the user sees why instead of an
    // unexplained SKIP. `Pass`/`Fail` use `output` for section dumps,
    // not notes, so this branch never fires for them.
    if outcome.status == TaskStatus::Skip && !outcome.output.is_empty() {
        reporter.note(&format!("{}: {}", check.label(), outcome.output));
    }
    outcome
}

/// Schedule every check under one [`thread::scope`] so the independent
/// cohort, the serial chain and coverage all overlap whenever Cargo's
/// per-`target/` lock allows it.
///
/// Layout (matches the README's `## Scheduling` diagram):
///
/// * Independent cohort: one worker thread per check, all in parallel.
/// * Serial chain: single worker walking
///   `compile → test → clippy → doc → doc-test`. Compile failure skips
///   the rest of the chain, since nothing else can build past it.
/// * Coverage: forks off the chain after `test` passes and runs in
///   parallel with the chain tail. Skipped when `test` did not pass.
///
/// Outcomes are returned in plan-insertion order so the verbose section
/// listing and the final summary stay deterministic.
///
/// A panicking check propagates the panic. Masking it as a `Fail` would
/// also drop the user's diagnostics.
fn run_pipeline(
    plan: &Plan,
    pbs: &[ProgressBar],
    coverage: Option<(&CoverageCheck, &ProgressBar)>,
    reporter: &Reporter,
    runner: &dyn Runner,
) -> (Vec<CheckOutcome>, Option<CheckOutcome>) {
    let mut outcomes: Vec<CheckOutcome> =
        (0..plan.len()).map(|_| CheckOutcome::skipped()).collect();

    let coverage_outcome = thread::scope(|s| {
        let independent_handles: Vec<_> = plan
            .independent()
            .map(|(idx, check)| {
                let pb = &pbs[idx];
                s.spawn(move || (idx, run_one(check, pb, reporter, runner)))
            })
            .collect();

        let chain_handle = s.spawn(move || {
            let mut chain_outcomes: Vec<(usize, CheckOutcome)> = Vec::new();
            let mut coverage_handle = None;
            let mut compile_failed = false;

            for (idx, check) in plan.serial_chain() {
                let pb = &pbs[idx];
                let label = check.label();
                let position = check.chain_position();
                let outcome = if compile_failed {
                    reporter.finish_spinner(pb, label, TaskStatus::Skip);
                    CheckOutcome::skipped()
                } else {
                    run_one(check, pb, reporter, runner)
                };
                let passed = outcome.passed();

                if position == Some(chain::COMPILE) && !passed {
                    compile_failed = true;
                }
                if position == Some(chain::TEST)
                    && passed
                    && let Some((cov, cov_pb)) = coverage
                {
                    coverage_handle = Some(s.spawn(move || run_one(cov, cov_pb, reporter, runner)));
                }
                chain_outcomes.push((idx, outcome));
            }

            // Coverage is only spawned when `test` passes. Otherwise
            // mark its spinner Skip so the user sees the gate did not
            // fire.
            let cov_outcome = coverage_handle
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
                })
                .or_else(|| {
                    coverage.map(|(cov, cov_pb)| {
                        reporter.finish_spinner(cov_pb, cov.label(), TaskStatus::Skip);
                        CheckOutcome::skipped()
                    })
                });

            (chain_outcomes, cov_outcome)
        });

        for handle in independent_handles {
            let (idx, outcome) = handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
            outcomes[idx] = outcome;
        }
        let (chain_outcomes, cov_outcome) = chain_handle
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
        for (idx, outcome) in chain_outcomes {
            outcomes[idx] = outcome;
        }
        cov_outcome
    });

    (outcomes, coverage_outcome)
}

/// Flatten plan outcomes plus optional coverage into `(label, outcome)`
/// pairs for reporting, in display order: plan items (insertion order),
/// then coverage if it ran.
fn flatten_outcomes<'a>(
    plan: &'a Plan,
    outcomes: &'a [CheckOutcome],
    coverage_outcome: Option<&'a CheckOutcome>,
) -> Vec<(&'a str, &'a CheckOutcome)> {
    let mut items: Vec<(&str, &CheckOutcome)> = Vec::new();
    for ((_, c), o) in plan.iter().zip(outcomes) {
        items.push((c.label(), o));
    }
    if let Some(o) = coverage_outcome {
        items.push((CoverageCheck::LABEL, o));
    }
    items
}

/// Print PASS sections (verbose only) then FAIL sections; return the
/// number of failing checks.
fn report_results(reporter: &Reporter, items: &[(&str, &CheckOutcome)]) -> usize {
    if reporter.is_verbose {
        for (label, outcome) in items {
            if outcome.passed() {
                reporter.print_section(label, &outcome.output, true);
            }
        }
    }
    for (label, outcome) in items {
        if outcome.failed() {
            reporter.print_section(label, &outcome.output, false);
        }
    }

    let failed: Vec<&str> = items
        .iter()
        .filter(|(_, o)| o.failed())
        .map(|(l, _)| *l)
        .collect();
    reporter.summary(items.len(), &failed);
    failed.len()
}
