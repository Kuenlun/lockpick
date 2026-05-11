// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::thread;

use indicatif::ProgressBar;

use crate::checks::{self, Check, coverage::CoverageCheck};
use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::error::LockpickError;
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling::{INSTALL_AUDIT, INSTALL_LLVM_COV, INSTALL_MACHETE, Toolchain};

pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::new(cli.verbose)?;

    let config = Config::load();
    let toolchain = Toolchain::detect();

    // Coverage runs by default; --skip coverage disables it, and --skip
    // test implicitly disables it because there are no .profraw files
    // to evaluate.
    let coverage_active = is_coverage_active(cli);

    require_tooling(cli, coverage_active, toolchain)?;
    if cli.skips(&SkipOption::Test) && !cli.skips(&SkipOption::Coverage) {
        reporter.info("--skip test implies coverage will be skipped");
    }

    let run_compile = !cli.skips(&SkipOption::Check);
    let parallel = checks::build_parallel(cli, coverage_active, toolchain, &config);
    let coverage_check = coverage_active.then_some(CoverageCheck {
        thresholds: config.coverage,
    });

    if !run_compile && parallel.is_empty() && coverage_check.is_none() {
        reporter.note("All checks disabled, nothing to run");
        return Ok(());
    }

    print_planned_commands(
        &reporter,
        run_compile,
        &parallel,
        coverage_check.as_ref().map(|c| c as &dyn Check),
    );

    // Create all spinners upfront so every stage is visible from the start.
    let compile_pb = run_compile.then(|| reporter.add_spinner("check"));
    let parallel_pbs: Vec<ProgressBar> = parallel
        .iter()
        .map(|c| reporter.add_spinner(c.label()))
        .collect();
    let coverage_pb = coverage_check
        .as_ref()
        .map(|c| reporter.add_spinner(c.label()));

    // Phase 1: compile gate.
    let compile_outcome = run_compile.then(|| {
        let outcome = checks::compile::CompileCheck.run();
        if let Some(pb) = &compile_pb {
            reporter.finish_spinner(pb, "check", outcome.status);
        }
        outcome
    });
    let compile_passed = compile_outcome.as_ref().is_none_or(CheckOutcome::passed);

    // Phase 2: parallel checks (only if compile passed).
    let parallel_outcomes: Vec<CheckOutcome> = if compile_passed {
        run_parallel(&parallel)
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
    let coverage_outcome = coverage_check.as_ref().map(|cov| {
        let outcome = if should_run_coverage_phase(compile_passed, &parallel, &parallel_outcomes) {
            cov.run()
        } else {
            CheckOutcome::skipped()
        };
        if let Some(pb) = &coverage_pb {
            reporter.finish_spinner(pb, "coverage", outcome.status);
        }
        outcome
    });

    let failure_count = report_results(
        &reporter,
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

fn run_parallel(checks: &[Box<dyn Check>]) -> Vec<CheckOutcome> {
    thread::scope(|s| {
        checks
            .iter()
            .map(|c| s.spawn(move || c.run()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| CheckOutcome {
                    status: TaskStatus::Fail,
                    output: String::new(),
                })
            })
            .collect()
    })
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

    // Collect failed labels for the footer.
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

    // Total visible checks (those that produced a spinner).
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
        let reporter = Reporter::new(false).unwrap();
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
        let reporter = Reporter::new(false).unwrap();
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
        let reporter = Reporter::new(false).unwrap();
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
        let reporter = Reporter::new(false).unwrap();
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
        // When compile fails, parallel outcomes are marked Skip but we
        // don't count the parallel positions as failures: only the compile
        // step is failing.
        let reporter = Reporter::new(false).unwrap();
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
        let reporter = Reporter::new(true).unwrap();
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
    fn report_results_works_without_compile_outcome_when_skipped() {
        let reporter = Reporter::new(false).unwrap();
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let outcomes = vec![pass("clippy")];
        // compile_passed=true reflects that compile was *skipped*, not failed.
        let n = report_results(&reporter, None, &parallel, &outcomes, true, None);
        assert_eq!(n, 0);
    }

    #[test]
    fn require_tooling_passes_when_every_tool_dependent_check_is_skipped() {
        let cli = cli_skipping(&[SkipOption::Machete, SkipOption::Audit, SkipOption::Coverage]);
        let toolchain = Toolchain::default();
        let result = require_tooling(&cli, false, toolchain);
        assert!(result.is_ok());
    }

    #[test]
    fn require_tooling_passes_when_every_tool_is_present() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain::all_present();
        let result = require_tooling(&cli, true, toolchain);
        assert!(result.is_ok());
    }

    #[test]
    fn require_tooling_errors_when_llvm_cov_missing_and_coverage_active() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain {
            llvm_cov: false,
            ..Toolchain::all_present()
        };
        let err = require_tooling(&cli, true, toolchain).unwrap_err();
        assert!(matches!(err, LockpickError::MissingTool { tool, .. } if tool == "cargo-llvm-cov"));
    }

    #[test]
    fn require_tooling_errors_when_machete_missing_and_not_skipped() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain {
            machete: false,
            ..Toolchain::all_present()
        };
        let err = require_tooling(&cli, false, toolchain).unwrap_err();
        assert!(matches!(err, LockpickError::MissingTool { tool, .. } if tool == "cargo-machete"));
    }

    #[test]
    fn require_tooling_errors_when_audit_missing_and_not_skipped() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain {
            audit: false,
            ..Toolchain::all_present()
        };
        let err = require_tooling(&cli, false, toolchain).unwrap_err();
        assert!(matches!(err, LockpickError::MissingTool { tool, .. } if tool == "cargo-audit"));
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
        let reporter = Reporter::new(false).unwrap();
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        // Just ensure it doesn't panic. Non-verbose path returns early.
        print_planned_commands(&reporter, true, &parallel, None);
    }

    #[test]
    fn print_planned_commands_prints_when_verbose() {
        let reporter = Reporter::new(true).unwrap();
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        // Smoke test: verbose path includes compile + parallel + coverage.
        print_planned_commands(
            &reporter,
            true,
            &parallel,
            Some(&CoverageCheck {
                thresholds: crate::config::CoverageConfig::default(),
            } as &dyn Check),
        );
    }
}
