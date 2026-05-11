// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::thread;

use indicatif::ProgressBar;

use crate::checks::{self, CargoCli, Check, Runner, coverage::CoverageCheck};
use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::error::LockpickError;
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling::{INSTALL_AUDIT, INSTALL_LLVM_COV, INSTALL_MACHETE, Toolchain};

/// Production entry point: builds the dependencies and delegates to the
/// pure orchestrator in [`run_with`].
pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::auto(cli.verbose);
    let toolchain = Toolchain::detect();
    let config = Config::load();
    let runner = CargoCli::detect();
    run_with(cli, &reporter, toolchain, &config, &runner)
}

/// Orchestrator with every collaborator injected so tests can drive
/// the full pipeline against fakes.
pub fn run_with(
    cli: &Cli,
    reporter: &Reporter,
    toolchain: Toolchain,
    config: &Config,
    runner: &dyn Runner,
) -> Result<(), LockpickError> {
    let coverage_active = is_coverage_active(cli);

    require_tooling(cli, coverage_active, toolchain)?;
    if cli.skips(&SkipOption::Test) && !cli.skips(&SkipOption::Coverage) {
        reporter.info("--skip test implies coverage will be skipped");
    }

    let run_compile = !cli.skips(&SkipOption::Check);
    let parallel = checks::build_parallel(cli, coverage_active, toolchain, config);
    let coverage_check = coverage_active.then_some(CoverageCheck {
        thresholds: config.coverage,
    });

    // `parallel.is_empty()` already implies `coverage_check.is_none()`
    // because coverage is only active when the `test` check is enabled
    // (which always lives in `parallel`).
    if !run_compile && parallel.is_empty() {
        reporter.note("All checks disabled, nothing to run");
        return Ok(());
    }

    print_planned_commands(
        reporter,
        run_compile,
        &parallel,
        coverage_check.as_ref().map(|c| c as &dyn Check),
    );

    let compile_pb = run_compile.then(|| reporter.add_spinner("check"));
    let parallel_pbs: Vec<ProgressBar> = parallel
        .iter()
        .map(|c| reporter.add_spinner(c.label()))
        .collect();
    let coverage_pb = coverage_check
        .as_ref()
        .map(|c| reporter.add_spinner(c.label()));

    // Phase 1: compile gate.
    let compile_outcome = compile_pb.map(|pb| {
        let outcome = checks::compile::CompileCheck.run(runner);
        reporter.finish_spinner(&pb, "check", outcome.status);
        outcome
    });
    let compile_passed = compile_outcome.as_ref().is_none_or(CheckOutcome::passed);

    // Phase 2: parallel checks (only if compile passed).
    let parallel_outcomes: Vec<CheckOutcome> = if compile_passed {
        run_parallel(&parallel, runner)
    } else {
        (0..parallel.len())
            .map(|_| CheckOutcome::skipped())
            .collect()
    };
    for ((check, outcome), pb) in parallel.iter().zip(&parallel_outcomes).zip(&parallel_pbs) {
        let status = if compile_passed {
            outcome.status
        } else {
            TaskStatus::Skip
        };
        reporter.finish_spinner(pb, check.label(), status);
    }

    // Phase 3: coverage gate (only if active and tests succeeded).
    let coverage_outcome = coverage_check.as_ref().zip(coverage_pb).map(|(cov, pb)| {
        let outcome = if should_run_coverage_phase(compile_passed, &parallel, &parallel_outcomes) {
            cov.run(runner)
        } else {
            CheckOutcome::skipped()
        };
        reporter.finish_spinner(&pb, "coverage", outcome.status);
        outcome
    });

    let failure_count = report_results(
        reporter,
        compile_outcome.as_ref(),
        &parallel,
        &parallel_outcomes,
        compile_passed,
        coverage_outcome.as_ref(),
    );

    if failure_count > 0 {
        return Err(LockpickError::ChecksFailed(failure_count));
    }

    Ok(())
}

/// Whether the coverage gate should run at all. The user can disable it
/// explicitly (`--skip coverage`) or implicitly by skipping tests.
fn is_coverage_active(cli: &Cli) -> bool {
    !cli.skips(&SkipOption::Coverage) && !cli.skips(&SkipOption::Test)
}

/// Fail fast if any enabled check requires an external cargo subcommand
/// that is not installed. Coverage, machete and audit are the only
/// tool-dependent checks in v1.
fn require_tooling(
    cli: &Cli,
    coverage_active: bool,
    toolchain: Toolchain,
) -> Result<(), LockpickError> {
    if coverage_active && !toolchain.llvm_cov {
        return Err(LockpickError::MissingTool {
            tool: "cargo-llvm-cov",
            install: INSTALL_LLVM_COV,
        });
    }
    if !cli.skips(&SkipOption::Machete) && !toolchain.machete {
        return Err(LockpickError::MissingTool {
            tool: "cargo-machete",
            install: INSTALL_MACHETE,
        });
    }
    if !cli.skips(&SkipOption::Audit) && !toolchain.audit {
        return Err(LockpickError::MissingTool {
            tool: "cargo-audit",
            install: INSTALL_AUDIT,
        });
    }
    Ok(())
}

/// Whether phase 3 should actually invoke the coverage check. Coverage
/// runs only when the compile gate and the `test` check both succeed —
/// otherwise the `.profraw` files are missing or stale.
fn should_run_coverage_phase(
    compile_passed: bool,
    parallel: &[Box<dyn Check>],
    outcomes: &[CheckOutcome],
) -> bool {
    compile_passed
        && parallel
            .iter()
            .zip(outcomes)
            .find(|(c, _)| c.label() == "test")
            .is_some_and(|(_, o)| o.passed())
}

fn print_planned_commands(
    reporter: &Reporter,
    run_compile: bool,
    parallel: &[Box<dyn Check>],
    coverage: Option<&dyn Check>,
) {
    if !reporter.is_verbose {
        return;
    }
    if run_compile {
        reporter.command(&checks::compile::CompileCheck.cmd());
    }
    for c in parallel {
        reporter.command(&c.cmd());
    }
    if let Some(c) = coverage {
        reporter.command(&c.cmd());
    }
    reporter.println("");
}

/// Spawn each check on its own scoped thread and collect the outcomes.
/// `thread::scope` captures any panic in `handle.join()` so a misbehaving
/// check is reported as `Fail` without taking the rest of the pipeline
/// down with it.
fn run_parallel(checks: &[Box<dyn Check>], runner: &dyn Runner) -> Vec<CheckOutcome> {
    thread::scope(|s| {
        checks
            .iter()
            .map(|c| s.spawn(move || c.run(runner)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap_or_else(|_| failed_outcome()))
            .collect()
    })
}

const fn failed_outcome() -> CheckOutcome {
    CheckOutcome {
        status: TaskStatus::Fail,
        output: String::new(),
    }
}

fn report_results(
    reporter: &Reporter,
    compile_outcome: Option<&CheckOutcome>,
    parallel: &[Box<dyn Check>],
    parallel_outcomes: &[CheckOutcome],
    compile_passed: bool,
    coverage_outcome: Option<&CheckOutcome>,
) -> usize {
    // PASS sections first when verbose so the operator can scan green-to-red.
    if reporter.is_verbose {
        if let Some(o) = compile_outcome
            && o.passed()
        {
            reporter.print_section("check", &o.output, TaskStatus::Pass);
        }
        for (check, outcome) in parallel.iter().zip(parallel_outcomes) {
            if outcome.passed() && compile_passed {
                reporter.print_section(check.label(), &outcome.output, TaskStatus::Pass);
            }
        }
        if let Some(o) = coverage_outcome
            && o.passed()
        {
            reporter.print_section("coverage", &o.output, TaskStatus::Pass);
        }
    }

    // FAIL sections.
    if let Some(o) = compile_outcome
        && o.failed()
    {
        reporter.print_section("check", &o.output, TaskStatus::Fail);
    }
    for (check, outcome) in parallel.iter().zip(parallel_outcomes) {
        if outcome.failed() && compile_passed {
            reporter.print_section(check.label(), &outcome.output, TaskStatus::Fail);
        }
    }
    if let Some(o) = coverage_outcome
        && o.failed()
    {
        reporter.print_section("coverage", &o.output, TaskStatus::Fail);
    }

    let mut failed: Vec<&str> = Vec::new();
    if let Some(o) = compile_outcome
        && o.failed()
    {
        failed.push("check");
    }
    if compile_passed {
        for (check, outcome) in parallel.iter().zip(parallel_outcomes) {
            if outcome.failed() {
                failed.push(check.label());
            }
        }
    }
    if let Some(o) = coverage_outcome
        && o.failed()
    {
        failed.push("coverage");
    }

    let mut total = parallel.len();
    if compile_outcome.is_some() {
        total += 1;
    }
    if coverage_outcome.is_some() {
        total += 1;
    }

    reporter.summary(total, &failed);

    failed.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::clippy::ClippyCheck;
    use crate::checks::fmt::FmtCheck;
    use crate::checks::{FakeRunner, SpawnResult};
    use crate::cli::SkipOption;

    fn pass(label: &str) -> CheckOutcome {
        CheckOutcome {
            status: TaskStatus::Pass,
            output: format!("{label} output"),
        }
    }

    fn fail(label: &str) -> CheckOutcome {
        CheckOutcome {
            status: TaskStatus::Fail,
            output: format!("{label} error"),
        }
    }

    fn cli_skipping(skips: &[SkipOption]) -> Cli {
        Cli {
            skip: skips.to_vec(),
            verbose: false,
        }
    }

    #[test]
    fn report_results_returns_zero_when_everything_passes() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck), Box::new(FmtCheck)];
        let outcomes = vec![pass("clippy"), pass("fmt")];
        let n = report_results(
            &reporter,
            Some(&pass("check")),
            &parallel,
            &outcomes,
            true,
            Some(&pass("coverage")),
        );
        assert_eq!(n, 0);
    }

    #[test]
    fn report_results_counts_a_failing_parallel_check() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck), Box::new(FmtCheck)];
        let outcomes = vec![pass("clippy"), fail("fmt")];
        let n = report_results(
            &reporter,
            Some(&pass("check")),
            &parallel,
            &outcomes,
            true,
            None,
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_counts_a_failing_compile_check() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![];
        let outcomes: Vec<CheckOutcome> = vec![];
        let n = report_results(
            &reporter,
            Some(&fail("check")),
            &parallel,
            &outcomes,
            false,
            None,
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_counts_a_failing_coverage_gate() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        let n = report_results(
            &reporter,
            Some(&pass("check")),
            &parallel,
            &outcomes,
            true,
            Some(&fail("coverage")),
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_does_not_count_parallel_when_compile_failed() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck), Box::new(FmtCheck)];
        let outcomes = vec![CheckOutcome::skipped(), CheckOutcome::skipped()];
        let n = report_results(
            &reporter,
            Some(&fail("check")),
            &parallel,
            &outcomes,
            false,
            None,
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_verbose_path_still_returns_correct_count() {
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        let n = report_results(
            &reporter,
            Some(&pass("check")),
            &parallel,
            &outcomes,
            true,
            Some(&pass("coverage")),
        );
        assert_eq!(n, 0);
    }

    #[test]
    fn report_results_verbose_emits_pass_and_fail_sections_for_coverage() {
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![];
        let outcomes: Vec<CheckOutcome> = vec![];
        let n = report_results(
            &reporter,
            Some(&pass("check")),
            &parallel,
            &outcomes,
            true,
            Some(&fail("coverage")),
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_works_without_compile_outcome_when_skipped() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        let n = report_results(&reporter, None, &parallel, &outcomes, true, None);
        assert_eq!(n, 0);
    }

    #[test]
    fn report_results_verbose_with_no_compile_or_coverage_outcome() {
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        let n = report_results(&reporter, None, &parallel, &outcomes, true, None);
        assert_eq!(n, 0);
    }

    #[test]
    fn report_results_verbose_with_failing_compile_skips_pass_section() {
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![CheckOutcome::skipped()];
        let n = report_results(
            &reporter,
            Some(&fail("check")),
            &parallel,
            &outcomes,
            false,
            None,
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_verbose_with_failing_coverage_skips_pass_section() {
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![fail("clippy")];
        let n = report_results(
            &reporter,
            Some(&pass("check")),
            &parallel,
            &outcomes,
            true,
            Some(&fail("coverage")),
        );
        assert_eq!(n, 2);
    }

    #[test]
    fn report_results_non_verbose_with_failing_coverage_only() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![];
        let outcomes: Vec<CheckOutcome> = vec![];
        let n = report_results(
            &reporter,
            None,
            &parallel,
            &outcomes,
            true,
            Some(&fail("coverage")),
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_non_verbose_with_passing_coverage_only() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![];
        let outcomes: Vec<CheckOutcome> = vec![];
        let n = report_results(
            &reporter,
            None,
            &parallel,
            &outcomes,
            true,
            Some(&pass("coverage")),
        );
        assert_eq!(n, 0);
    }

    #[test]
    fn report_results_with_compile_failed_and_parallel_outcomes_doesnt_double_count() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        // outcome.failed() is true, but compile_passed=false → parallel section
        // is skipped in report_results. Only the compile failure counts.
        let outcomes = vec![fail("clippy")];
        let n = report_results(
            &reporter,
            Some(&fail("check")),
            &parallel,
            &outcomes,
            false,
            None,
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn report_results_verbose_skips_parallel_pass_section_when_compile_failed() {
        // Synthetic case: compile_passed=false but parallel outcomes were
        // synthesised as passing. The verbose pass-section branch must
        // still short-circuit on the compile_passed guard.
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        let n = report_results(
            &reporter,
            Some(&fail("check")),
            &parallel,
            &outcomes,
            false,
            None,
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn require_tooling_passes_when_every_tool_dependent_check_is_skipped() {
        let cli = cli_skipping(&[SkipOption::Machete, SkipOption::Audit, SkipOption::Coverage]);
        let toolchain = Toolchain::default();
        assert!(require_tooling(&cli, false, toolchain).is_ok());
    }

    #[test]
    fn require_tooling_passes_when_every_tool_is_present() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain::all_present();
        assert!(require_tooling(&cli, true, toolchain).is_ok());
    }

    #[test]
    fn require_tooling_errors_when_llvm_cov_missing_and_coverage_active() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain {
            llvm_cov: false,
            ..Toolchain::all_present()
        };
        let err = require_tooling(&cli, true, toolchain).unwrap_err();
        assert!(err.to_string().contains("cargo-llvm-cov"));
    }

    #[test]
    fn require_tooling_errors_when_machete_missing_and_not_skipped() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain {
            machete: false,
            ..Toolchain::all_present()
        };
        let err = require_tooling(&cli, false, toolchain).unwrap_err();
        assert!(err.to_string().contains("cargo-machete"));
    }

    #[test]
    fn require_tooling_errors_when_audit_missing_and_not_skipped() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain {
            audit: false,
            ..Toolchain::all_present()
        };
        let err = require_tooling(&cli, false, toolchain).unwrap_err();
        assert!(err.to_string().contains("cargo-audit"));
    }

    #[test]
    fn is_coverage_active_is_false_when_user_skips_coverage() {
        let cli = cli_skipping(&[SkipOption::Coverage]);
        assert!(!is_coverage_active(&cli));
    }

    #[test]
    fn is_coverage_active_is_false_when_user_skips_test() {
        let cli = cli_skipping(&[SkipOption::Test]);
        assert!(!is_coverage_active(&cli));
    }

    #[test]
    fn is_coverage_active_is_true_by_default() {
        let cli = cli_skipping(&[]);
        assert!(is_coverage_active(&cli));
    }

    #[test]
    fn should_run_coverage_phase_requires_compile_passed() {
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        assert!(!should_run_coverage_phase(false, &parallel, &outcomes));
    }

    #[test]
    fn should_run_coverage_phase_requires_test_outcome_passed() {
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(crate::checks::test::TestCheck {
            instrumented: false,
            nextest: false,
        })];
        let outcomes_pass = vec![pass("test")];
        let outcomes_fail = vec![fail("test")];
        assert!(should_run_coverage_phase(true, &parallel, &outcomes_pass));
        assert!(!should_run_coverage_phase(true, &parallel, &outcomes_fail));
    }

    #[test]
    fn should_run_coverage_phase_is_false_without_a_test_check() {
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        assert!(!should_run_coverage_phase(true, &parallel, &outcomes));
    }

    #[test]
    fn print_planned_commands_is_no_op_when_verbose_is_false() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        print_planned_commands(&reporter, true, &parallel, None);
    }

    #[test]
    fn print_planned_commands_prints_when_verbose() {
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let coverage = CoverageCheck {
            thresholds: crate::config::CoverageConfig::default(),
        };
        print_planned_commands(&reporter, true, &parallel, Some(&coverage as &dyn Check));
    }

    #[test]
    fn print_planned_commands_skips_compile_banner_when_compile_disabled() {
        let reporter = Reporter::new(true, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        print_planned_commands(&reporter, false, &parallel, None);
    }

    #[test]
    fn run_parallel_executes_each_check_and_collects_outcomes() {
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck), Box::new(FmtCheck)];
        let fake = FakeRunner::with_responses(vec![
            Ok(SpawnResult {
                success: true,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            Ok(SpawnResult {
                success: false,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
        ]);
        let outcomes = run_parallel(&parallel, &fake);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().any(CheckOutcome::passed));
        assert!(outcomes.iter().any(CheckOutcome::failed));
    }

    /// Runner that always panics when spawned. Used to exercise the
    /// `run_parallel` panic-recovery branch without introducing a custom
    /// `Check` fixture whose `label()`/`cmd()` methods would only exist
    /// to satisfy the trait.
    struct PanickingRunner;
    impl Runner for PanickingRunner {
        fn spawn(
            &self,
            _sub: &str,
            _args: &[&str],
            _envs: &[(&str, &str)],
        ) -> std::io::Result<SpawnResult> {
            panic!("simulated runner panic");
        }
    }

    #[test]
    fn run_parallel_replaces_panicking_threads_with_fail_outcomes() {
        // Suppress libtest's panic backtrace for the expected panic so
        // the test output stays clean.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = run_parallel(&parallel, &PanickingRunner);
        std::panic::set_hook(prev);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].failed());
        assert!(outcomes[0].output.is_empty());
    }

    #[test]
    fn failed_outcome_helper_is_fail_with_empty_output() {
        let o = failed_outcome();
        assert!(o.failed());
        assert!(o.output.is_empty());
    }

    fn passing_runner() -> FakeRunner {
        // Enough canned responses to cover every cargo call across all
        // phases (compile + parallel + coverage).
        let mut responses = Vec::new();
        for _ in 0..32 {
            responses.push(Ok(SpawnResult {
                success: true,
                stdout: br#"{ "data": [{ "files": [{}], "totals": {
                    "functions": { "count": 1, "covered": 1 },
                    "lines": { "count": 1, "covered": 1 },
                    "regions": { "count": 1, "covered": 1 },
                    "branches": { "count": 1, "covered": 1 }
                } }] }"#
                    .to_vec(),
                stderr: Vec::new(),
            }));
        }
        FakeRunner::with_responses(responses)
    }

    #[test]
    fn run_with_succeeds_when_every_check_passes() {
        let reporter = Reporter::new(true, false);
        let cli = Cli {
            skip: vec![SkipOption::Doc, SkipOption::DocTest, SkipOption::License],
            verbose: true,
        };
        let runner = passing_runner();
        let toolchain = Toolchain::all_present();
        let config = Config::default();
        assert!(run_with(&cli, &reporter, toolchain, &config, &runner).is_ok());
    }

    #[test]
    fn run_with_returns_checks_failed_when_a_check_fails() {
        let reporter = Reporter::new(false, false);
        let cli = Cli {
            skip: vec![
                SkipOption::DocTest,
                SkipOption::License,
                SkipOption::Doc,
                SkipOption::Audit,
                SkipOption::Machete,
                SkipOption::Coverage,
                SkipOption::Test,
            ],
            verbose: false,
        };
        // Single failing response for the lone compile check.
        let runner = FakeRunner::with_responses(vec![Ok(SpawnResult {
            success: false,
            stdout: b"compile error".to_vec(),
            stderr: Vec::new(),
        })]);
        let toolchain = Toolchain::all_present();
        let config = Config::default();
        let err = run_with(&cli, &reporter, toolchain, &config, &runner).unwrap_err();
        assert!(err.to_string().contains("check(s) failed"));
    }

    #[test]
    fn run_with_returns_missing_tool_error() {
        let reporter = Reporter::new(false, false);
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let toolchain = Toolchain::default(); // nothing present
        let config = Config::default();
        let runner = FakeRunner::passing();
        let err = run_with(&cli, &reporter, toolchain, &config, &runner).unwrap_err();
        assert!(err.to_string().contains("required tool"));
    }

    #[test]
    fn run_with_emits_info_when_test_skipped_but_coverage_not_skipped() {
        let reporter = Reporter::new(true, false);
        let cli = Cli {
            skip: vec![
                SkipOption::Test,
                SkipOption::Machete,
                SkipOption::Audit,
                SkipOption::DocTest,
                SkipOption::License,
            ],
            verbose: true,
        };
        let runner = passing_runner();
        let toolchain = Toolchain::all_present();
        let config = Config::default();
        assert!(run_with(&cli, &reporter, toolchain, &config, &runner).is_ok());
    }

    #[test]
    fn run_with_proceeds_when_only_compile_is_skipped() {
        let reporter = Reporter::new(false, false);
        let cli = Cli {
            skip: vec![
                SkipOption::Check,
                SkipOption::DocTest,
                SkipOption::License,
                SkipOption::Doc,
                SkipOption::Audit,
                SkipOption::Machete,
                SkipOption::Coverage,
                SkipOption::Test,
            ],
            verbose: false,
        };
        // Clippy + Fmt remain in `parallel`, so the "all checks disabled"
        // shortcut must not fire. The runner proceeds and the fake runner
        // reports them as passing.
        let runner = passing_runner();
        let toolchain = Toolchain::all_present();
        let config = Config::default();
        assert!(run_with(&cli, &reporter, toolchain, &config, &runner).is_ok());
    }

    #[test]
    fn run_with_reports_all_disabled_path() {
        let reporter = Reporter::new(false, false);
        let cli = Cli {
            skip: vec![
                SkipOption::Check,
                SkipOption::Clippy,
                SkipOption::Fmt,
                SkipOption::Test,
                SkipOption::DocTest,
                SkipOption::Doc,
                SkipOption::Machete,
                SkipOption::Audit,
                SkipOption::License,
                SkipOption::Coverage,
            ],
            verbose: false,
        };
        let runner = FakeRunner::passing();
        let toolchain = Toolchain::all_present();
        let config = Config::default();
        assert!(run_with(&cli, &reporter, toolchain, &config, &runner).is_ok());
    }

    #[test]
    fn run_with_skips_coverage_phase_when_test_fails() {
        let reporter = Reporter::new(false, false);
        let cli = Cli {
            skip: vec![
                SkipOption::DocTest,
                SkipOption::License,
                SkipOption::Doc,
                SkipOption::Audit,
                SkipOption::Machete,
                SkipOption::Clippy,
                SkipOption::Fmt,
            ],
            verbose: false,
        };
        // Phase 1 compile passes; phase 2 single `test` check fails;
        // phase 3 coverage should be skipped (and the report counts the
        // single test failure).
        let runner = FakeRunner::with_responses(vec![
            Ok(SpawnResult {
                success: true,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            Ok(SpawnResult {
                success: false,
                stdout: b"tests failed".to_vec(),
                stderr: Vec::new(),
            }),
        ]);
        let toolchain = Toolchain::all_present();
        let config = Config::default();
        let err = run_with(&cli, &reporter, toolchain, &config, &runner).unwrap_err();
        assert!(err.to_string().contains("1 check(s) failed"), "got: {err}");
    }

    #[test]
    fn run_with_skips_parallel_when_compile_fails() {
        let reporter = Reporter::new(false, false);
        let cli = Cli {
            skip: vec![
                SkipOption::DocTest,
                SkipOption::License,
                SkipOption::Doc,
                SkipOption::Audit,
                SkipOption::Machete,
                SkipOption::Coverage,
                SkipOption::Test,
            ],
            verbose: false,
        };
        // Compile fails -> parallel checks marked Skip, no coverage.
        let runner = FakeRunner::with_responses(vec![Ok(SpawnResult {
            success: false,
            stdout: b"compile error".to_vec(),
            stderr: Vec::new(),
        })]);
        let toolchain = Toolchain::all_present();
        let config = Config::default();
        let err = run_with(&cli, &reporter, toolchain, &config, &runner).unwrap_err();
        assert!(err.to_string().contains("1 check(s) failed"), "got: {err}");
    }
}
