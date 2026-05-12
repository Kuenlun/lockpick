// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::thread;

use indicatif::ProgressBar;

use crate::checks::{self, CargoCli, Check, Runner, coverage::CoverageCheck};
use crate::cli::{Cli, SkipOption};
use crate::config::{Config, LockpickMetadata};
use crate::error::LockpickError;
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling::{INSTALL_AUDIT, INSTALL_LLVM_COV, INSTALL_MACHETE, Toolchain};

/// Production entry point: builds the dependencies and delegates to the
/// pure orchestrator in [`run_with`].
pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::auto(cli.verbose);
    let toolchain = Toolchain::detect();
    let metadata = LockpickMetadata::load();
    let runner = CargoCli::detect();
    run_with(
        cli,
        &reporter,
        toolchain,
        &metadata.config,
        metadata.has_lib_target,
        &runner,
    )
}

/// Orchestrator with every collaborator injected so tests can drive
/// the full pipeline against fakes.
pub fn run_with(
    cli: &Cli,
    reporter: &Reporter,
    toolchain: Toolchain,
    config: &Config,
    has_lib: bool,
    runner: &dyn Runner,
) -> Result<(), LockpickError> {
    let coverage_active = is_coverage_active(cli);

    require_tooling(cli, coverage_active, toolchain)?;
    if cli.skips(&SkipOption::Test) && !cli.skips(&SkipOption::Coverage) {
        reporter.note("--skip test implies coverage will be skipped");
    }

    let run_compile = !cli.skips(&SkipOption::Check);
    let parallel = checks::build_parallel(cli, coverage_active, toolchain, config, has_lib);
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

    if reporter.is_verbose {
        print_planned_commands(
            reporter,
            run_compile,
            &parallel,
            coverage_check.as_ref().map(|c| c as &dyn Check),
        );
    }

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
        std::iter::repeat_with(CheckOutcome::skipped)
            .take(parallel.len())
            .collect()
    };
    for ((check, outcome), pb) in parallel.iter().zip(&parallel_outcomes).zip(&parallel_pbs) {
        reporter.finish_spinner(pb, check.label(), outcome.status);
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

    let items = flatten_outcomes(
        compile_outcome.as_ref(),
        &parallel,
        &parallel_outcomes,
        coverage_outcome.as_ref(),
    );
    let failure_count = report_results(reporter, &items);

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

/// Caller has already gated on `reporter.is_verbose`; print one banner
/// line per planned cargo invocation, plus a trailing blank line so the
/// spinners start on a fresh row.
fn print_planned_commands(
    reporter: &Reporter,
    run_compile: bool,
    parallel: &[Box<dyn Check>],
    coverage: Option<&dyn Check>,
) {
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
            .map(|handle| {
                handle.join().unwrap_or_else(|_| CheckOutcome {
                    status: TaskStatus::Fail,
                    output: String::new(),
                })
            })
            .collect()
    })
}

/// Build the flat list of `(label, outcome)` pairs used by reporting.
/// Pulling this out keeps the orchestrator's data flow obvious and lets
/// the single-pass [`report_results`] stay branch-free.
fn flatten_outcomes<'a>(
    compile_outcome: Option<&'a CheckOutcome>,
    parallel: &'a [Box<dyn Check>],
    parallel_outcomes: &'a [CheckOutcome],
    coverage_outcome: Option<&'a CheckOutcome>,
) -> Vec<(&'a str, &'a CheckOutcome)> {
    let mut items: Vec<(&str, &CheckOutcome)> = Vec::new();
    if let Some(o) = compile_outcome {
        items.push(("check", o));
    }
    for (c, o) in parallel.iter().zip(parallel_outcomes) {
        items.push((c.label(), o));
    }
    if let Some(o) = coverage_outcome {
        items.push(("coverage", o));
    }
    items
}

/// Print PASS sections (verbose only) and FAIL sections in two passes
/// over the flat item list. Returns the number of failing checks.
fn report_results(reporter: &Reporter, items: &[(&str, &CheckOutcome)]) -> usize {
    if reporter.is_verbose {
        for (label, outcome) in items {
            if outcome.passed() {
                reporter.print_section(label, &outcome.output, TaskStatus::Pass);
            }
        }
    }
    for (label, outcome) in items {
        if outcome.failed() {
            reporter.print_section(label, &outcome.output, TaskStatus::Fail);
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
        let reporter = Reporter::new(true, false);
        let compile = pass("check");
        let clippy = pass("clippy");
        let coverage = pass("coverage");
        let items = vec![
            ("check", &compile),
            ("clippy", &clippy),
            ("coverage", &coverage),
        ];
        assert_eq!(report_results(&reporter, &items), 0);
    }

    #[test]
    fn report_results_counts_every_failure_and_emits_fail_sections() {
        let reporter = Reporter::new(false, false);
        let compile = pass("check");
        let fmt_fail = fail("fmt");
        let cov_fail = fail("coverage");
        let items = vec![
            ("check", &compile),
            ("fmt", &fmt_fail),
            ("coverage", &cov_fail),
        ];
        assert_eq!(report_results(&reporter, &items), 2);
    }

    #[test]
    fn report_results_ignores_skipped_outcomes() {
        let reporter = Reporter::new(true, false);
        let compile = fail("check");
        let skipped = CheckOutcome::skipped();
        let items = vec![("check", &compile), ("clippy", &skipped)];
        assert_eq!(report_results(&reporter, &items), 1);
    }

    #[test]
    fn report_results_on_empty_items_prints_ok_with_zero_total() {
        let reporter = Reporter::new(false, false);
        assert_eq!(report_results(&reporter, &[]), 0);
    }

    #[test]
    fn flatten_outcomes_drops_none_outcomes_and_preserves_parallel_order() {
        let compile = pass("check");
        let clippy = pass("clippy");
        let fmt = fail("fmt");
        let coverage = pass("coverage");
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck), Box::new(FmtCheck)];
        let outcomes = vec![clippy, fmt];
        let items = flatten_outcomes(Some(&compile), &parallel, &outcomes, Some(&coverage));
        assert_eq!(
            items.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            vec!["check", "clippy", "fmt", "coverage"]
        );

        // Drop the compile/coverage edges to cover the `None` arms.
        let no_compile = flatten_outcomes(None, &parallel, &outcomes, None);
        assert_eq!(
            no_compile.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            vec!["clippy", "fmt"]
        );
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
    fn print_planned_commands_prints_compile_parallel_and_coverage() {
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
    /// `run_parallel` panic-recovery branch.
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
        assert!(
            run_with(
                &cli,
                &reporter,
                Toolchain::all_present(),
                &Config::default(),
                false,
                &runner,
            )
            .is_ok()
        );
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
        let err = run_with(
            &cli,
            &reporter,
            Toolchain::all_present(),
            &Config::default(),
            false,
            &runner,
        )
        .unwrap_err();
        assert!(err.to_string().contains("check(s) failed"));
    }

    #[test]
    fn run_with_returns_missing_tool_error() {
        let reporter = Reporter::new(false, false);
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        // Toolchain::default() reports nothing present.
        let runner = FakeRunner::passing();
        let err = run_with(
            &cli,
            &reporter,
            Toolchain::default(),
            &Config::default(),
            false,
            &runner,
        )
        .unwrap_err();
        assert!(err.to_string().contains("required tool"));
    }

    #[test]
    fn run_with_emits_note_when_test_skipped_but_coverage_not_skipped() {
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
        assert!(
            run_with(
                &cli,
                &reporter,
                Toolchain::all_present(),
                &Config::default(),
                false,
                &runner,
            )
            .is_ok()
        );
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
        let runner = passing_runner();
        assert!(
            run_with(
                &cli,
                &reporter,
                Toolchain::all_present(),
                &Config::default(),
                false,
                &runner,
            )
            .is_ok()
        );
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
        assert!(
            run_with(
                &cli,
                &reporter,
                Toolchain::all_present(),
                &Config::default(),
                false,
                &runner,
            )
            .is_ok()
        );
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
        // phase 3 coverage is skipped (and the single test failure is
        // reflected in the error).
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
        let err = run_with(
            &cli,
            &reporter,
            Toolchain::all_present(),
            &Config::default(),
            false,
            &runner,
        )
        .unwrap_err();
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
        let err = run_with(
            &cli,
            &reporter,
            Toolchain::all_present(),
            &Config::default(),
            false,
            &runner,
        )
        .unwrap_err();
        assert!(err.to_string().contains("1 check(s) failed"), "got: {err}");
    }
}
