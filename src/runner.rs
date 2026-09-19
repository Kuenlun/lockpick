// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::io::IsTerminal;
use std::thread;

use indicatif::ProgressBar;

use crate::checks::{self, CargoCli, Check, Plan, Runner, chain, coverage::CoverageCheck};
use crate::cli::{Cli, SkipOption};
use crate::config::{Config, CoverageConfig, LockpickMetadata};
use crate::error::{LockpickError, MissingTool};
use crate::fix;
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling::{self, ColorMode, Tool, Toolchain};

/// Run the full check pipeline. Loads tooling, config and workspace
/// metadata, then orchestrates the independent cohort, the serial chain
/// and coverage.
pub(crate) fn run(mut cli: Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::auto(cli.verbose);
    let toolchain = Toolchain::detect();
    let metadata = LockpickMetadata::load()?;
    // Fold any `skip = [...]` from Cargo.toml into the CLI's view of
    // skips so every downstream consumer reads from a single source.
    cli.merge_config_skips(&metadata.config.skip);
    // Single color decision shared by our output and every subprocess
    // (`CARGO_TERM_COLOR`, rustfmt `--color`) so `--color`/`NO_COLOR`/TTY
    // signals land coherently across both.
    let color = cli.color_mode(std::io::stdout().is_terminal());
    // Process-wide override: every other crate linked in inherits it.
    colored::control::set_override(color == ColorMode::Always);
    let is_nightly = tooling::is_nightly();
    let config = &metadata.config;
    let has_lib = metadata.has_lib_target;

    let coverage_active = is_coverage_active(&cli, config);

    require_coverage_consistency(&cli)?;
    require_tooling(&cli, coverage_active, &toolchain)?;
    require_nightly_for_branches(coverage_active, config, is_nightly)?;

    let coverage_host = if coverage_active && config.host_target {
        Some(tooling::coverage_host()?)
    } else {
        None
    };
    let runner = CargoCli::detect(
        color,
        metadata.workspace_root.clone(),
        metadata.target_directory.as_deref(),
    )
    .with_coverage_host(coverage_host);
    let options = checks::util::BuildOptions {
        locked: config.locked,
        host_target: config.host_target,
    };

    // Fix phase runs first so the same invocation can heal the tree
    // and then prove it. Abort on failure: the pipeline would only
    // refail on the same lint.
    if cli.fix && fix::apply(&cli, &runner, &reporter, options).is_err() {
        return Err(LockpickError::ChecksFailed(1));
    }

    // `-Z coverage-options=branch` is nightly-only. Stable runs still
    // get functions, lines and regions.
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
    let coverage_check = coverage_active.then(|| CoverageCheck {
        options,
        thresholds: config.coverage.unwrap_or_default(),
        branch_coverage,
    });

    // Coverage rides on `test` (the only source of `.profraw` files),
    // so an empty plan must imply no coverage check.
    if plan.is_empty() {
        debug_assert!(
            coverage_check.is_none(),
            "invariant: empty `plan` must imply no coverage check"
        );
        return Err(LockpickError::NoChecksToRun);
    }

    if cli.skips(SkipOption::Coverage) && config.coverage.is_none() {
        reporter.note("--skip coverage has no effect: coverage is opt-in and not configured");
    }
    if config.coverage.is_some() && cli.skips(SkipOption::Test) && !cli.skips(SkipOption::Coverage)
    {
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
            coverage_check.as_ref().map(|c| -> &dyn Check { c }),
        );
    }

    let (outcomes, coverage_outcome) =
        run_pipeline(&plan, coverage_check.as_ref(), &reporter, &runner);

    let items = flatten_outcomes(&plan, &outcomes, coverage_outcome.as_ref());
    let failure_count = report_results(&reporter, &items);

    if failure_count > 0 {
        return Err(LockpickError::ChecksFailed(failure_count));
    }

    Ok(())
}

/// Whether the coverage gate runs. Opt-in via `--coverage` or the
/// `[*.metadata.lockpick.coverage]` table, disabled by `--skip coverage`
/// or by `--skip test` (no instrumentation, no coverage).
fn is_coverage_active(cli: &Cli, config: &Config) -> bool {
    (cli.coverage || config.coverage.is_some())
        && !cli.skips(SkipOption::Coverage)
        && !cli.skips(SkipOption::Test)
}

/// Refuse contradictory coverage flags. `--coverage` demands the gate
/// run, so combining it with a skip of `coverage` or `test` (from the
/// CLI or the config `skip` list) is a usage error, not a silent win
/// for either side. Runs after [`Cli::merge_config_skips`] so config
/// entries are covered too.
fn require_coverage_consistency(cli: &Cli) -> Result<(), LockpickError> {
    if cli.coverage {
        for skip in [SkipOption::Coverage, SkipOption::Test] {
            if cli.skips(skip) {
                return Err(LockpickError::CoverageConflict(skip.skip_flag()));
            }
        }
    }
    Ok(())
}

/// Refuse to run when `coverage.branches` is configured on stable.
/// Silently dropping the threshold would mask the user's explicit ask,
/// and branch coverage needs nightly's `-Z coverage-options=branch`.
const fn require_nightly_for_branches(
    coverage_active: bool,
    config: &Config,
    is_nightly: bool,
) -> Result<(), LockpickError> {
    let branches_configured = matches!(
        config.coverage,
        Some(CoverageConfig {
            branches: Some(_),
            ..
        })
    );
    if coverage_active && branches_configured && !is_nightly {
        Err(LockpickError::BranchesRequireNightly)
    } else {
        Ok(())
    }
}

/// Collect every absent cargo subcommand at once so the user can
/// install all of them in a single `cargo install …` invocation.
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

/// Render one banner line per planned cargo invocation. Caller gates
/// on `is_verbose`.
fn print_planned_commands(reporter: &Reporter, plan: &Plan, coverage: Option<&dyn Check>) {
    for (_, c) in plan.iter() {
        reporter.command(&c.cmd());
    }
    if let Some(c) = coverage {
        reporter.command(&c.cmd());
    }
    reporter.diagln("");
}

/// Run a single check and finish its progress bar from the same
/// thread, so PASS/FAIL marks land as soon as the check ends.
fn run_one(
    check: &dyn Check,
    pb: &ProgressBar,
    reporter: &Reporter,
    runner: &dyn Runner,
) -> CheckOutcome {
    let outcome = check.run(runner);
    reporter.finish_spinner(pb, check.label(), outcome.status);
    outcome
}

/// Schedule every check under one [`thread::scope`] so the independent
/// cohort, the serial chain and coverage all overlap whenever Cargo's
/// per-`target/` lock allows it. Layout mirrors the README's
/// `## How it schedules` diagram:
///
/// * Independent cohort: one worker thread per check, all in parallel.
/// * Serial chain: single worker walking
///   `compile, test, clippy, doc, doc-test`. Compile failure skips the
///   rest of the chain.
/// * Coverage: forks off after `test` passes and runs in parallel with
///   the chain tail.
///
/// Outcomes return in plan-insertion order so verbose sections and the
/// summary are deterministic. Panicking checks propagate via
/// [`std::panic::resume_unwind`] rather than masking as `Fail`.
fn run_pipeline(
    plan: &Plan,
    coverage_check: Option<&CoverageCheck>,
    reporter: &Reporter,
    runner: &dyn Runner,
) -> (Vec<CheckOutcome>, Option<CheckOutcome>) {
    // Create spinners in display order before partitioning execution cohorts.
    let (independent, mut chain): (Vec<_>, Vec<_>) = plan
        .iter()
        .map(|(index, check)| (index, check, reporter.add_spinner(check.label())))
        .partition(|(_, check, _)| check.chain_position().is_none());
    chain.sort_by_key(|(_, check, _)| check.chain_position());
    let coverage_pb = coverage_check.map(|check| reporter.add_spinner(check.label()));
    let coverage = coverage_check.zip(coverage_pb.as_ref());
    let mut outcomes = Vec::with_capacity(plan.len());

    let coverage_outcome = thread::scope(|s| {
        let independent_handles: Vec<_> = independent
            .iter()
            .map(|(idx, check, pb)| s.spawn(move || (*idx, run_one(*check, pb, reporter, runner))))
            .collect();

        let chain_handle = s.spawn(move || {
            let mut chain_outcomes: Vec<(usize, CheckOutcome)> = Vec::new();
            let mut coverage_handle = None;
            let mut compile_failed = false;

            for (idx, check, pb) in &chain {
                let label = check.label();
                let position = check.chain_position();
                let outcome = if compile_failed {
                    reporter.finish_spinner(pb, label, TaskStatus::Skip);
                    CheckOutcome::skipped()
                } else {
                    run_one(*check, pb, reporter, runner)
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
                chain_outcomes.push((*idx, outcome));
            }

            // Coverage only spawns when `test` passes. Otherwise mark
            // its spinner Skip so the user sees the gate did not fire.
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
            outcomes.push((idx, outcome));
        }
        let (chain_outcomes, cov_outcome) = chain_handle
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
        outcomes.extend(chain_outcomes);
        cov_outcome
    });

    outcomes.sort_by_key(|(index, _)| *index);
    (
        outcomes.into_iter().map(|(_, outcome)| outcome).collect(),
        coverage_outcome,
    )
}

/// Flatten plan outcomes plus optional coverage into `(label, outcome)`
/// pairs for reporting, in insertion order with coverage last.
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

/// Print PASS sections (verbose only) then FAIL sections. Return the
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use clap::Parser;

    use super::*;

    fn config_with_coverage() -> Config {
        Config {
            coverage: Some(CoverageConfig::default()),
            ..Config::default()
        }
    }

    #[test]
    fn coverage_is_inactive_by_default() {
        let cli = Cli::parse_from(["lockpick"]);
        assert!(!is_coverage_active(&cli, &Config::default()));
    }

    #[test]
    fn coverage_activates_via_flag_or_config_table() {
        let flag = Cli::parse_from(["lockpick", "--coverage"]);
        assert!(is_coverage_active(&flag, &Config::default()));

        let plain = Cli::parse_from(["lockpick"]);
        assert!(is_coverage_active(&plain, &config_with_coverage()));
    }

    #[test]
    fn skipping_coverage_or_test_deactivates_a_configured_gate() {
        let config = config_with_coverage();
        for skip in ["coverage", "test"] {
            let cli = Cli::parse_from(["lockpick", "--skip", skip]);
            assert!(!is_coverage_active(&cli, &config), "skip {skip}");
        }
    }

    #[test]
    fn coverage_flag_with_contradicting_skip_is_a_usage_error() {
        for skip in ["coverage", "test"] {
            let cli = Cli::parse_from(["lockpick", "--coverage", "--skip", skip]);
            assert!(
                matches!(
                    require_coverage_consistency(&cli),
                    Err(LockpickError::CoverageConflict(flag)) if flag == skip
                ),
                "skip {skip} must conflict with --coverage"
            );
        }
    }

    #[test]
    fn coverage_flag_alone_is_consistent() {
        let cli = Cli::parse_from(["lockpick", "--coverage"]);
        assert!(require_coverage_consistency(&cli).is_ok());
        let plain = Cli::parse_from(["lockpick", "--skip", "coverage"]);
        assert!(require_coverage_consistency(&plain).is_ok());
    }

    #[test]
    fn require_tooling_skips_llvm_cov_when_coverage_inactive()
    -> Result<(), Box<dyn std::error::Error>> {
        // An empty toolchain is missing every optional tool; with
        // machete and audit skipped and coverage inactive, nothing is
        // required. Activating coverage must then demand cargo-llvm-cov.
        let cli = Cli::parse_from(["lockpick", "--skip", "machete,audit"]);
        let toolchain = Toolchain::default();
        assert!(require_tooling(&cli, false, &toolchain).is_ok());

        let Err(LockpickError::MissingTools(missing)) = require_tooling(&cli, true, &toolchain)
        else {
            return Err("active coverage must require cargo-llvm-cov".into());
        };
        assert_eq!(missing.len(), 1);
        assert_eq!(missing.first().unwrap().binary, "cargo-llvm-cov");
        Ok(())
    }

    #[test]
    fn branches_threshold_requires_nightly_only_when_coverage_active() {
        let config = Config {
            coverage: Some(CoverageConfig {
                branches: Some(100),
                ..CoverageConfig::default()
            }),
            ..Config::default()
        };
        assert!(matches!(
            require_nightly_for_branches(true, &config, false),
            Err(LockpickError::BranchesRequireNightly)
        ));
        assert!(require_nightly_for_branches(true, &config, true).is_ok());
        assert!(require_nightly_for_branches(false, &config, false).is_ok());
        assert!(require_nightly_for_branches(true, &Config::default(), false).is_ok());
    }
    struct PipelineRunner {
        fail: &'static str,
        calls: std::sync::Mutex<Vec<&'static str>>,
        fmt_started: std::sync::mpsc::Sender<()>,
        wait_for_fmt: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        compile_started: std::sync::mpsc::Sender<()>,
        wait_for_compile: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl PipelineRunner {
        fn new(fail: &'static str) -> Self {
            let (fmt_started, wait_for_fmt) = std::sync::mpsc::channel();
            let (compile_started, wait_for_compile) = std::sync::mpsc::channel();
            Self {
                fail,
                calls: std::sync::Mutex::new(Vec::new()),
                fmt_started,
                wait_for_fmt: std::sync::Mutex::new(wait_for_fmt),
                compile_started,
                wait_for_compile: std::sync::Mutex::new(wait_for_compile),
            }
        }
    }

    impl Runner for PipelineRunner {
        fn spawn(
            &self,
            sub: &str,
            args: &[&str],
            _envs: &[(&str, &str)],
        ) -> std::io::Result<checks::runner::SpawnResult> {
            let label = match sub {
                "check" => "check",
                "fmt" => "fmt",
                "clippy" => "clippy",
                "doc" => "doc",
                "test" if args.contains(&"--doc") => "doc-test",
                "test" => "test",
                _ => return Err(std::io::Error::other("unexpected pipeline command")),
            };
            let timeout = std::time::Duration::from_secs(2);
            match label {
                "check" => {
                    self.compile_started
                        .send(())
                        .map_err(std::io::Error::other)?;
                    self.wait_for_fmt
                        .lock()
                        .unwrap()
                        .recv_timeout(timeout)
                        .map_err(std::io::Error::other)?;
                }
                "fmt" => {
                    self.fmt_started.send(()).map_err(std::io::Error::other)?;
                    self.wait_for_compile
                        .lock()
                        .unwrap()
                        .recv_timeout(timeout)
                        .map_err(std::io::Error::other)?;
                }
                _ => {}
            }
            self.calls.lock().unwrap().push(label);
            Ok(checks::runner::SpawnResult {
                success: label != self.fail,
                stdout: label.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn pipeline_overlaps_independent_work_and_preserves_serial_and_report_order() {
        let cli = Cli::parse_from(["lockpick", "--skip", "machete,audit"]);
        let plan = checks::build_plan(
            &cli,
            false,
            &Toolchain::default(),
            &Config::default(),
            true,
            false,
            ColorMode::Never,
        );
        let runner = PipelineRunner::new("");
        let (outcomes, coverage) = run_pipeline(&plan, None, &Reporter::auto(false), &runner);
        assert!(coverage.is_none());
        for ((_, check), outcome) in plan.iter().zip(&outcomes) {
            assert!(outcome.passed(), "{}", outcome.output);
            assert_eq!(outcome.output.trim(), check.label());
        }
        let serial: Vec<_> = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|label| *label != "fmt")
            .collect();
        assert_eq!(serial, ["check", "test", "clippy", "doc", "doc-test"]);
    }

    #[test]
    fn compile_failure_skips_the_chain_and_test_failure_skips_only_coverage() {
        let cli = Cli::parse_from(["lockpick", "--skip", "machete,audit"]);
        let plan = checks::build_plan(
            &cli,
            false,
            &Toolchain::default(),
            &Config::default(),
            true,
            false,
            ColorMode::Never,
        );
        let coverage = CoverageCheck {
            options: checks::util::BuildOptions::default(),
            thresholds: CoverageConfig::default(),
            branch_coverage: false,
        };
        for fail in ["check", "test"] {
            let runner = PipelineRunner::new(fail);
            let (outcomes, coverage) =
                run_pipeline(&plan, Some(&coverage), &Reporter::auto(false), &runner);
            assert_eq!(coverage.unwrap().status, TaskStatus::Skip);
            for ((_, check), outcome) in plan.iter().zip(&outcomes) {
                let expected = if check.label() == fail {
                    TaskStatus::Fail
                } else if fail == "check" && check.label() != "fmt" {
                    TaskStatus::Skip
                } else {
                    TaskStatus::Pass
                };
                assert_eq!(
                    outcome.status,
                    expected,
                    "{} after {fail} failure",
                    check.label()
                );
            }
        }
    }
}
