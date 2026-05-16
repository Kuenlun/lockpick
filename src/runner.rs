// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::thread;

use indicatif::ProgressBar;

use crate::checks::{self, CargoCli, Check, Plan, Runner, chain, coverage::CoverageCheck};
use crate::cli::{Cli, SkipOption};
use crate::config::{Config, LockpickMetadata};
use crate::error::{LockpickError, MissingTool};
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling::{Tool, Toolchain};

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

    let plan = checks::build_plan(cli, coverage_active, toolchain, config, has_lib);
    let coverage_check = coverage_active.then_some(CoverageCheck {
        thresholds: config.coverage,
    });

    // Coverage rides on `test`, which is the only path that emits the
    // profraw files coverage consumes. If `test` did not survive the
    // CLI, coverage cannot have either — the assert pins that invariant
    // for future refactors.
    if plan.is_empty() {
        debug_assert!(
            coverage_check.is_none(),
            "invariant: empty `plan` must imply no coverage check"
        );
        reporter.note("All checks disabled, nothing to run");
        return Ok(());
    }

    if cli.skips(&SkipOption::Test) && !cli.skips(&SkipOption::Coverage) {
        reporter.note("--skip test implies coverage will be skipped");
    }

    if reporter.is_verbose {
        print_planned_commands(
            reporter,
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

    let (outcomes, coverage_outcome) = run_pipeline(&plan, &pbs, coverage, reporter, runner);

    let items = flatten_outcomes(&plan, &outcomes, coverage_outcome.as_ref());
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
    if !cli.skips(&SkipOption::Machete) && !toolchain.has(Tool::Machete) {
        missing.push(MissingTool {
            binary: "cargo-machete",
            skip_flag: SkipOption::Machete.skip_flag(),
        });
    }
    if !cli.skips(&SkipOption::Audit) && !toolchain.has(Tool::Audit) {
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
    reporter.println("");
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
    outcome
}

/// Schedule every check under one [`thread::scope`] so the independent
/// cohort, the serial chain and coverage all overlap whenever Cargo's
/// per-`target/` lock allows it.
///
/// Layout (matches the README's `## Scheduling` diagram):
///
/// * Independent cohort — one worker thread per check, all in parallel.
/// * Serial chain — single worker walking
///   `compile → test → clippy → doc → doc-test`. Compile failure skips
///   the rest of the chain, since nothing else can build past it.
/// * Coverage — forks off the chain after `test` passes and runs in
///   parallel with the chain tail; skipped when `test` did not pass.
///
/// Outcomes are returned in plan-insertion order so the verbose section
/// listing and the final summary stay deterministic.
///
/// A panicking check propagates the panic — masking it as a `Fail` would
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

            // Coverage is only spawned when `test` passes; otherwise
            // mark its spinner Skip so the user sees the gate did not
            // fire. `or_else` keeps the two cases in their natural
            // order — primary first, fallback second.
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
    use crate::checks::compile::CompileCheck;
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

    /// Test runner that keys its response off the cargo subcommand name
    /// instead of a FIFO of canned responses. Necessary for parallel
    /// pipeline tests where the order in which workers reach `spawn` is
    /// non-deterministic.
    struct ByCommandRunner {
        fail: &'static [&'static str],
    }

    impl Runner for ByCommandRunner {
        fn spawn(
            &self,
            sub: &str,
            _args: &[&str],
            _envs: &[(&str, &str)],
        ) -> std::io::Result<SpawnResult> {
            Ok(SpawnResult {
                success: !self.fail.contains(&sub),
                stdout: sub.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    fn pbs_for(plan: &Plan, reporter: &Reporter) -> Vec<ProgressBar> {
        plan.iter()
            .map(|(_, c)| reporter.add_spinner(c.label()))
            .collect()
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
    fn flatten_outcomes_preserves_plan_order_and_appends_coverage() {
        let compile = pass("check");
        let fmt = fail("fmt");
        let coverage = pass("coverage");
        let plan = Plan::from_items(vec![Box::new(CompileCheck), Box::new(FmtCheck)]);
        let outcomes = vec![compile, fmt];
        let items = flatten_outcomes(&plan, &outcomes, Some(&coverage));
        assert_eq!(
            items.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            vec!["check", "fmt", "coverage"]
        );

        let no_coverage = flatten_outcomes(&plan, &outcomes, None);
        assert_eq!(
            no_coverage.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            vec!["check", "fmt"]
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
    fn require_tooling_bundles_every_missing_tool_into_one_error() {
        let cli = cli_skipping(&[]);
        let toolchain = Toolchain::default();
        let err = require_tooling(&cli, true, &toolchain).unwrap_err();
        let msg = err.to_string();
        // Drip-feed regression: every absent tool must be reported in
        // a single error, not just the first one that fails the check.
        for binary in ["cargo-llvm-cov", "cargo-machete", "cargo-audit"] {
            assert!(msg.contains(binary), "missing `{binary}` in error: {msg}");
        }
        // …and the install hint must combine them into one cargo invocation.
        assert!(
            msg.contains("cargo install cargo-llvm-cov cargo-machete cargo-audit"),
            "expected combined install line in error: {msg}"
        );
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
    fn print_planned_commands_prints_every_plan_check_and_coverage() {
        let reporter = Reporter::new(true, false);
        let plan = Plan::from_items(vec![Box::new(CompileCheck), Box::new(ClippyCheck)]);
        let coverage = CoverageCheck {
            thresholds: crate::config::CoverageConfig::default(),
        };
        print_planned_commands(&reporter, &plan, Some(&coverage as &dyn Check));
    }

    #[test]
    fn print_planned_commands_omits_coverage_banner_when_coverage_inactive() {
        let reporter = Reporter::new(true, false);
        let plan = Plan::from_items(vec![Box::new(ClippyCheck)]);
        print_planned_commands(&reporter, &plan, None);
    }

    #[test]
    #[should_panic = "simulated check panic"]
    fn run_pipeline_re_raises_a_panic_from_the_independent_cohort() {
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
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let reporter = Reporter::new(false, false);
        // FmtCheck is independent; this exercises the indep-cohort join.
        let plan = Plan::from_items(vec![Box::new(FmtCheck)]);
        let pbs = pbs_for(&plan, &reporter);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pipeline(&plan, &pbs, None, &reporter, &PanickingRunner)
        }));
        std::panic::set_hook(prev);
        match result {
            Ok(_) => panic!("expected panic, run_pipeline returned Ok"),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[test]
    #[should_panic = "simulated check panic"]
    fn run_pipeline_re_raises_a_panic_from_the_serial_chain() {
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
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let reporter = Reporter::new(false, false);
        // ClippyCheck is in the chain; this exercises the chain join.
        let plan = Plan::from_items(vec![Box::new(ClippyCheck)]);
        let pbs = pbs_for(&plan, &reporter);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pipeline(&plan, &pbs, None, &reporter, &PanickingRunner)
        }));
        std::panic::set_hook(prev);
        match result {
            Ok(_) => panic!("expected panic, run_pipeline returned Ok"),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[test]
    #[should_panic = "simulated coverage panic"]
    fn run_pipeline_re_raises_a_panic_from_the_coverage_thread() {
        // Coverage's invocation is `cargo llvm-cov report …`; instrumented
        // `test` also calls `llvm-cov` (without `report`). Discriminate
        // on the first arg so only the coverage worker panics — that
        // exercises the coverage-handle join.
        struct PanicOnCoverageReportRunner;
        impl Runner for PanicOnCoverageReportRunner {
            fn spawn(
                &self,
                sub: &str,
                args: &[&str],
                _envs: &[(&str, &str)],
            ) -> std::io::Result<SpawnResult> {
                assert!(
                    !(sub == "llvm-cov" && args.first() == Some(&"report")),
                    "simulated coverage panic",
                );
                Ok(SpawnResult {
                    success: true,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                })
            }
        }
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let reporter = Reporter::new(false, false);
        let plan = Plan::from_items(vec![
            Box::new(CompileCheck),
            Box::new(crate::checks::test::TestCheck {
                instrumented: true,
                nextest: false,
            }),
        ]);
        let pbs = pbs_for(&plan, &reporter);
        let cov_check = CoverageCheck {
            thresholds: crate::config::CoverageConfig::default(),
        };
        let cov_pb = reporter.add_spinner(cov_check.label());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pipeline(
                &plan,
                &pbs,
                Some((&cov_check, &cov_pb)),
                &reporter,
                &PanicOnCoverageReportRunner,
            )
        }));
        std::panic::set_hook(prev);
        match result {
            Ok(_) => panic!("expected panic, run_pipeline returned Ok"),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[test]
    fn run_pipeline_executes_every_check_and_returns_outcomes_in_plan_order() {
        let reporter = Reporter::new(false, false);
        // Mixed plan: chain (clippy, doc, check) interleaved with
        // independent (fmt, audit). Chain order (`check → clippy → doc`)
        // is enforced by chain_position and is independent of the plan's
        // insertion order, which is what the returned outcomes follow.
        let plan = Plan::from_items(vec![
            Box::new(ClippyCheck),
            Box::new(FmtCheck),
            Box::new(DocCheck),
            Box::new(AuditCheck),
            Box::new(CompileCheck),
        ]);
        let pbs = pbs_for(&plan, &reporter);

        let runner = ByCommandRunner { fail: &["doc"] };
        let (outcomes, cov) = run_pipeline(&plan, &pbs, None, &reporter, &runner);
        assert_eq!(outcomes.len(), 5);
        assert!(cov.is_none());
        let labels: Vec<&str> = plan.iter().map(|(_, c)| c.label()).collect();
        assert_eq!(
            labels,
            vec!["clippy", "fmt", "doc", "audit", "check"],
            "plan iteration order drifted"
        );
        assert_eq!(
            outcomes.iter().map(|o| o.status).collect::<Vec<_>>(),
            vec![
                TaskStatus::Pass, // clippy
                TaskStatus::Pass, // fmt
                TaskStatus::Fail, // doc
                TaskStatus::Pass, // audit
                TaskStatus::Pass, // check
            ],
            "outcomes are not in plan order"
        );
        assert!(pbs.iter().all(ProgressBar::is_finished));
    }

    #[test]
    fn run_pipeline_walks_serial_chain_in_canonical_order() {
        let reporter = Reporter::new(false, false);
        // Inserted out of canonical order; serial chain must still drive
        // them as `check → test → clippy → doc → doc test`.
        let plan = Plan::from_items(vec![
            Box::new(DocCheck),
            Box::new(crate::checks::test::TestCheck {
                instrumented: false,
                nextest: false,
            }),
            Box::new(ClippyCheck),
            Box::new(CompileCheck),
        ]);
        let pbs = pbs_for(&plan, &reporter);
        let fake = FakeRunner::with_responses((0..plan.len()).map(|_| Ok(ok_spawn())).collect());
        let _ = run_pipeline(&plan, &pbs, None, &reporter, &fake);
        let calls = fake.calls.lock().unwrap().clone();
        let subs: Vec<&str> = calls.iter().map(|c| c.sub.as_str()).collect();
        assert_eq!(
            subs,
            vec!["check", "test", "clippy", "doc"],
            "serial chain lost canonical order"
        );
    }

    #[test]
    fn run_pipeline_short_circuits_chain_when_compile_fails_but_independent_still_runs() {
        let reporter = Reporter::new(false, false);
        let plan = Plan::from_items(vec![
            Box::new(CompileCheck),
            Box::new(ClippyCheck),
            Box::new(DocCheck),
            Box::new(crate::checks::test::TestCheck {
                instrumented: false,
                nextest: false,
            }),
            Box::new(FmtCheck),
            Box::new(AuditCheck),
        ]);
        let pbs = pbs_for(&plan, &reporter);
        let runner = ByCommandRunner { fail: &["check"] };
        let (outcomes, cov) = run_pipeline(&plan, &pbs, None, &reporter, &runner);
        assert!(cov.is_none());
        let by_label: std::collections::HashMap<&str, TaskStatus> = plan
            .iter()
            .zip(outcomes.iter())
            .map(|((_, c), o)| (c.label(), o.status))
            .collect();
        assert_eq!(by_label["check"], TaskStatus::Fail);
        // Rest of the chain is skipped — nothing else compiles past it.
        assert_eq!(by_label["clippy"], TaskStatus::Skip);
        assert_eq!(by_label["doc"], TaskStatus::Skip);
        assert_eq!(by_label["test"], TaskStatus::Skip);
        // Independent cohort is unaffected by compile failure.
        assert_eq!(by_label["fmt"], TaskStatus::Pass);
        assert_eq!(by_label["audit"], TaskStatus::Pass);
    }

    #[test]
    fn run_pipeline_forks_coverage_after_test_passes_and_runs_it_in_parallel_with_chain_tail() {
        let reporter = Reporter::new(false, false);
        let plan = Plan::from_items(vec![
            Box::new(CompileCheck),
            Box::new(crate::checks::test::TestCheck {
                instrumented: true,
                nextest: false,
            }),
            Box::new(ClippyCheck),
        ]);
        let pbs = pbs_for(&plan, &reporter);
        let cov_check = CoverageCheck {
            thresholds: crate::config::CoverageConfig::default(),
        };
        let cov_pb = reporter.add_spinner(cov_check.label());

        let runner = CoverageReportRunner;
        let (outcomes, cov_outcome) =
            run_pipeline(&plan, &pbs, Some((&cov_check, &cov_pb)), &reporter, &runner);

        let cov = cov_outcome.expect("coverage was forked after test passed");
        assert!(cov.passed(), "coverage outcome: {}", cov.output);
        for o in outcomes {
            assert!(o.passed());
        }
        assert!(cov_pb.is_finished());
    }

    #[test]
    fn run_pipeline_skips_coverage_when_test_fails_and_marks_its_spinner_skip() {
        let reporter = Reporter::new(false, false);
        let plan = Plan::from_items(vec![
            Box::new(CompileCheck),
            Box::new(crate::checks::test::TestCheck {
                instrumented: true,
                nextest: false,
            }),
        ]);
        let pbs = pbs_for(&plan, &reporter);
        let cov_check = CoverageCheck {
            thresholds: crate::config::CoverageConfig::default(),
        };
        let cov_pb = reporter.add_spinner(cov_check.label());

        let runner = ByCommandRunner {
            fail: &["test", "llvm-cov"],
        };
        let (_, cov_outcome) =
            run_pipeline(&plan, &pbs, Some((&cov_check, &cov_pb)), &reporter, &runner);

        let cov = cov_outcome.expect("coverage entry returned even when skipped");
        assert!(matches!(cov.status, TaskStatus::Skip));
        assert!(cov_pb.is_finished());
    }

    #[test]
    fn run_pipeline_skips_coverage_when_compile_fails_so_test_never_runs() {
        let reporter = Reporter::new(false, false);
        let plan = Plan::from_items(vec![
            Box::new(CompileCheck),
            Box::new(crate::checks::test::TestCheck {
                instrumented: true,
                nextest: false,
            }),
        ]);
        let pbs = pbs_for(&plan, &reporter);
        let cov_check = CoverageCheck {
            thresholds: crate::config::CoverageConfig::default(),
        };
        let cov_pb = reporter.add_spinner(cov_check.label());

        let runner = ByCommandRunner { fail: &["check"] };
        let (_, cov_outcome) =
            run_pipeline(&plan, &pbs, Some((&cov_check, &cov_pb)), &reporter, &runner);

        let cov = cov_outcome.expect("coverage entry returned even when skipped");
        assert!(matches!(cov.status, TaskStatus::Skip));
    }

    #[test]
    fn run_pipeline_short_circuits_cleanly_on_an_empty_plan() {
        let reporter = Reporter::new(false, false);
        let plan = Plan::from_items(Vec::new());
        let pbs: Vec<ProgressBar> = Vec::new();
        let fake = FakeRunner::with_responses(Vec::new());
        let (outcomes, cov) = run_pipeline(&plan, &pbs, None, &reporter, &fake);
        assert!(outcomes.is_empty());
        assert!(cov.is_none());
    }

    #[test]
    fn run_one_finishes_the_spinner_and_returns_the_outcome() {
        let reporter = Reporter::new(false, false);
        let pb = reporter.add_spinner("clippy");
        let fake = FakeRunner::passing();
        let outcome = run_one(&ClippyCheck, &pb, &reporter, &fake);
        assert!(outcome.passed());
        assert!(pb.is_finished());
    }

    fn ok_spawn() -> SpawnResult {
        SpawnResult {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    /// Production-grade fake runner for the full pipeline: cargo
    /// subcommands all succeed; `llvm-cov report` returns a 100%
    /// coverage JSON so the coverage gate also passes.
    struct CoverageReportRunner;

    impl Runner for CoverageReportRunner {
        fn spawn(
            &self,
            sub: &str,
            args: &[&str],
            _envs: &[(&str, &str)],
        ) -> std::io::Result<SpawnResult> {
            let stdout = if sub == "llvm-cov" && args.first() == Some(&"report") {
                br#"{ "data": [{ "files": [{}], "totals": {
                    "functions": { "count": 1, "covered": 1 },
                    "lines": { "count": 1, "covered": 1 },
                    "regions": { "count": 1, "covered": 1 },
                    "branches": { "count": 1, "covered": 1 }
                } }] }"#
                    .to_vec()
            } else {
                Vec::new()
            };
            Ok(SpawnResult {
                success: true,
                stdout,
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn run_with_succeeds_when_every_check_passes() {
        let reporter = Reporter::new(true, false);
        let cli = Cli {
            skip: vec![SkipOption::Doc, SkipOption::License],
            verbose: true,
        };
        assert!(
            run_with(
                &cli,
                &reporter,
                &Toolchain::all_present(),
                &Config::default(),
                true,
                &CoverageReportRunner,
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
                SkipOption::Clippy,
                SkipOption::Fmt,
            ],
            verbose: false,
        };
        let runner = ByCommandRunner { fail: &["check"] };
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
        assert!(
            run_with(
                &cli,
                &reporter,
                &Toolchain::all_present(),
                &Config::default(),
                false,
                &CoverageReportRunner,
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
        assert!(
            run_with(
                &cli,
                &reporter,
                &Toolchain::all_present(),
                &Config::default(),
                false,
                &CoverageReportRunner,
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
        let runner = ByCommandRunner {
            fail: &["test", "llvm-cov"],
        };
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
    fn run_with_skips_chain_tail_when_compile_fails() {
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
                SkipOption::Clippy,
                SkipOption::Fmt,
            ],
            verbose: false,
        };
        let runner = ByCommandRunner { fail: &["check"] };
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
