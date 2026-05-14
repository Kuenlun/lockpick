// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::thread;

use indicatif::ProgressBar;

use crate::checks::{
    self, CargoCli, Check, Runner, compile::CompileCheck, coverage::CoverageCheck, test::TestCheck,
};
use crate::cli::{Cli, SkipOption};
use crate::config::{Config, LockpickMetadata};
use crate::error::LockpickError;
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling::{INSTALL_AUDIT, INSTALL_LLVM_COV, INSTALL_MACHETE, Tool, Toolchain};

/// Resolve runtime dependencies and delegate to [`run_with`].
#[cfg_attr(test, allow(dead_code))]
pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::auto(cli.verbose);
    let toolchain = Toolchain::detect();
    let metadata = LockpickMetadata::load();
    let runner = CargoCli::detect();
    run_with(
        cli,
        &reporter,
        &toolchain,
        &metadata.config,
        metadata.has_lib_target,
        &runner,
    )
}

/// Orchestrate the full check pipeline with every collaborator injected.
pub fn run_with(
    cli: &Cli,
    reporter: &Reporter,
    toolchain: &Toolchain,
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

    // Coverage rides on `test`, which always lives in `parallel`; the
    // assert pins that invariant for future refactors.
    if !run_compile && parallel.is_empty() {
        debug_assert!(
            coverage_check.is_none(),
            "invariant: empty `parallel` must imply no coverage check"
        );
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

    let compile = CompileCheck;
    let compile_pb = run_compile.then(|| reporter.add_spinner(compile.label()));
    let parallel_pbs: Vec<ProgressBar> = parallel
        .iter()
        .map(|c| reporter.add_spinner(c.label()))
        .collect();
    let coverage_pb = coverage_check
        .as_ref()
        .map(|c| reporter.add_spinner(c.label()));

    // Phase 1 — compile gate.
    let compile_outcome = compile_pb.map(|pb| {
        let outcome = compile.run(runner);
        reporter.finish_spinner(&pb, compile.label(), outcome.status);
        outcome
    });
    let compile_passed = compile_outcome.as_ref().is_none_or(CheckOutcome::passed);

    // Phase 2 — parallel checks. Skipped wholesale if compile failed.
    let parallel_outcomes: Vec<CheckOutcome> = if compile_passed {
        run_parallel(&parallel, &parallel_pbs, reporter, runner)
    } else {
        for (check, pb) in parallel.iter().zip(&parallel_pbs) {
            reporter.finish_spinner(pb, check.label(), TaskStatus::Skip);
        }
        std::iter::repeat_with(CheckOutcome::skipped)
            .take(parallel.len())
            .collect()
    };

    // Phase 3 — coverage gate, only when tests passed.
    let coverage_outcome = coverage_check.as_ref().zip(coverage_pb).map(|(cov, pb)| {
        let outcome = if should_run_coverage_phase(compile_passed, &parallel, &parallel_outcomes) {
            cov.run(runner)
        } else {
            CheckOutcome::skipped()
        };
        reporter.finish_spinner(&pb, cov.label(), outcome.status);
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

/// Whether the coverage gate runs. Disabled by `--skip coverage` or by
/// `--skip test` (no instrumentation, no coverage).
fn is_coverage_active(cli: &Cli) -> bool {
    !cli.skips(&SkipOption::Coverage) && !cli.skips(&SkipOption::Test)
}

/// Fail fast when an enabled check needs an absent cargo subcommand.
fn require_tooling(
    cli: &Cli,
    coverage_active: bool,
    toolchain: &Toolchain,
) -> Result<(), LockpickError> {
    if coverage_active && !toolchain.has(Tool::LlvmCov) {
        return Err(LockpickError::MissingTool {
            tool: "cargo-llvm-cov",
            install: INSTALL_LLVM_COV,
        });
    }
    if !cli.skips(&SkipOption::Machete) && !toolchain.has(Tool::Machete) {
        return Err(LockpickError::MissingTool {
            tool: "cargo-machete",
            install: INSTALL_MACHETE,
        });
    }
    if !cli.skips(&SkipOption::Audit) && !toolchain.has(Tool::Audit) {
        return Err(LockpickError::MissingTool {
            tool: "cargo-audit",
            install: INSTALL_AUDIT,
        });
    }
    Ok(())
}

/// Coverage runs only when compile and `test` both succeeded, else the
/// `.profraw` files are absent or stale.
fn should_run_coverage_phase(
    compile_passed: bool,
    parallel: &[Box<dyn Check>],
    outcomes: &[CheckOutcome],
) -> bool {
    compile_passed
        && parallel
            .iter()
            .zip(outcomes)
            .find(|(c, _)| c.label() == TestCheck::LABEL)
            .is_some_and(|(_, o)| o.passed())
}

/// Render one banner line per planned cargo invocation, plus a trailing
/// blank line. Caller is responsible for the `is_verbose` gate.
fn print_planned_commands(
    reporter: &Reporter,
    run_compile: bool,
    parallel: &[Box<dyn Check>],
    coverage: Option<&dyn Check>,
) {
    if run_compile {
        reporter.command(&CompileCheck.cmd());
    }
    for c in parallel {
        reporter.command(&c.cmd());
    }
    if let Some(c) = coverage {
        reporter.command(&c.cmd());
    }
    reporter.println("");
}

/// Run every check on its own scoped thread and collect outcomes in
/// input order. Each spinner is finished from inside its worker so
/// PASS/FAIL marks land progressively rather than in one batch.
///
/// A panicking check propagates the panic — masking it as a `Fail`
/// would also drop the user's diagnostics.
fn run_parallel(
    checks: &[Box<dyn Check>],
    pbs: &[ProgressBar],
    reporter: &Reporter,
    runner: &dyn Runner,
) -> Vec<CheckOutcome> {
    thread::scope(|s| {
        checks
            .iter()
            .zip(pbs)
            .map(|(check, pb)| {
                s.spawn(move || {
                    let outcome = check.run(runner);
                    reporter.finish_spinner(pb, check.label(), outcome.status);
                    outcome
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| match handle.join() {
                Ok(outcome) => outcome,
                Err(payload) => std::panic::resume_unwind(payload),
            })
            .collect()
    })
}

/// Flatten the three phases into `(label, outcome)` pairs for reporting.
fn flatten_outcomes<'a>(
    compile_outcome: Option<&'a CheckOutcome>,
    parallel: &'a [Box<dyn Check>],
    parallel_outcomes: &'a [CheckOutcome],
    coverage_outcome: Option<&'a CheckOutcome>,
) -> Vec<(&'a str, &'a CheckOutcome)> {
    let mut items: Vec<(&str, &CheckOutcome)> = Vec::new();
    if let Some(o) = compile_outcome {
        items.push((CompileCheck::LABEL, o));
    }
    for (c, o) in parallel.iter().zip(parallel_outcomes) {
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
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::checks::audit::AuditCheck;
    use crate::checks::clippy::ClippyCheck;
    use crate::checks::doc::DocCheck;
    use crate::checks::doctest::DocTestCheck;
    use crate::checks::fmt::FmtCheck;
    use crate::checks::license_header::LicenseHeaderCheck;
    use crate::checks::machete::MacheteCheck;
    use crate::checks::{FakeRunner, SpawnResult};
    use crate::cli::SkipOption;
    use crate::reporter::LABEL_WIDTH;

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
    fn every_check_label_fits_inside_label_width() {
        let labels: Vec<&'static str> = vec![
            CompileCheck.label(),
            ClippyCheck.label(),
            FmtCheck.label(),
            crate::checks::test::TestCheck {
                instrumented: false,
                nextest: false,
            }
            .label(),
            DocTestCheck.label(),
            DocCheck.label(),
            MacheteCheck.label(),
            AuditCheck.label(),
            LicenseHeaderCheck {
                header_path: std::path::PathBuf::new(),
                globs: Vec::new(),
            }
            .label(),
            CoverageCheck {
                thresholds: crate::config::CoverageConfig::default(),
            }
            .label(),
        ];
        for l in &labels {
            assert!(
                l.len() <= LABEL_WIDTH,
                "label `{l}` ({len} chars) exceeds LABEL_WIDTH = {LABEL_WIDTH}",
                len = l.len(),
            );
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
        assert!(require_tooling(&cli, false, &toolchain).is_ok());
    }

    #[test]
    fn require_tooling_passes_when_every_tool_is_present() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain::all_present();
        assert!(require_tooling(&cli, true, &toolchain).is_ok());
    }

    #[test]
    fn require_tooling_errors_when_llvm_cov_missing_and_coverage_active() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain::all_present().without(Tool::LlvmCov);
        let err = require_tooling(&cli, true, &toolchain).unwrap_err();
        assert!(err.to_string().contains("cargo-llvm-cov"));
    }

    #[test]
    fn require_tooling_errors_when_machete_missing_and_not_skipped() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain::all_present().without(Tool::Machete);
        let err = require_tooling(&cli, false, &toolchain).unwrap_err();
        assert!(err.to_string().contains("cargo-machete"));
    }

    #[test]
    fn require_tooling_errors_when_audit_missing_and_not_skipped() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain::all_present().without(Tool::Audit);
        let err = require_tooling(&cli, false, &toolchain).unwrap_err();
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
    #[should_panic = "simulated check panic"]
    fn run_parallel_re_raises_a_panicking_check_thread() {
        struct PanickingRunner;
        impl Runner for PanickingRunner {
            fn spawn(
                &self,
                _sub: &str,
                _args: &[&str],
                _envs: &[(&str, &str)],
            ) -> std::io::Result<SpawnResult> {
                panic!("simulated check panic");
            }
        }
        // Suppress libtest's backtrace so the expected panic stays quiet.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck)];
        let pbs: Vec<ProgressBar> = parallel
            .iter()
            .map(|c| reporter.add_spinner(c.label()))
            .collect();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_parallel(&parallel, &pbs, &reporter, &PanickingRunner)
        }));
        std::panic::set_hook(prev);
        match result {
            Ok(_) => panic!("expected panic, run_parallel returned Ok"),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[test]
    fn run_parallel_executes_each_check_and_collects_outcomes() {
        let reporter = Reporter::new(false, false);
        let parallel: Vec<Box<dyn Check>> = vec![Box::new(ClippyCheck), Box::new(FmtCheck)];
        let pbs: Vec<ProgressBar> = parallel
            .iter()
            .map(|c| reporter.add_spinner(c.label()))
            .collect();
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
        let outcomes = run_parallel(&parallel, &pbs, &reporter, &fake);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().any(CheckOutcome::passed));
        assert!(outcomes.iter().any(CheckOutcome::failed));
        assert!(pbs.iter().all(ProgressBar::is_finished));
    }

    fn passing_runner() -> FakeRunner {
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
            skip: vec![SkipOption::Doc, SkipOption::License],
            verbose: true,
        };
        let runner = passing_runner();
        assert!(
            run_with(
                &cli,
                &reporter,
                &Toolchain::all_present(),
                &Config::default(),
                true,
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
        let runner = FakeRunner::with_responses(vec![Ok(SpawnResult {
            success: false,
            stdout: b"compile error".to_vec(),
            stderr: Vec::new(),
        })]);
        let err = run_with(
            &cli,
            &reporter,
            &Toolchain::all_present(),
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
        let runner = FakeRunner::passing();
        let err = run_with(
            &cli,
            &reporter,
            &Toolchain::default(),
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
                &Toolchain::all_present(),
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
                &Toolchain::all_present(),
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
                &Toolchain::all_present(),
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
            &Toolchain::all_present(),
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
        let runner = FakeRunner::with_responses(vec![Ok(SpawnResult {
            success: false,
            stdout: b"compile error".to_vec(),
            stderr: Vec::new(),
        })]);
        let err = run_with(
            &cli,
            &reporter,
            &Toolchain::all_present(),
            &Config::default(),
            false,
            &runner,
        )
        .unwrap_err();
        assert!(err.to_string().contains("1 check(s) failed"), "got: {err}");
    }
}
