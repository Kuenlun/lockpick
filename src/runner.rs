// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::thread;

use indicatif::ProgressBar;

use crate::checks::{self, Check};
use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::error::LockpickError;
use crate::reporter::{CheckOutcome, Reporter, TaskStatus};
use crate::tooling;

pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::new()?;
    crate::logger::init(cli.verbose, &reporter.mp, reporter.is_tty);

    let _config = Config::load();

    let run_compile = !cli.skips(&SkipOption::Check);
    let has_llvm_cov = cli.opt_in.coverage && tooling::has_llvm_cov();
    let parallel = checks::build_parallel(cli, has_llvm_cov);

    if !run_compile && parallel.is_empty() {
        log::info!("All checks disabled, nothing to run");
        return Ok(());
    }

    // Create all spinners upfront so every stage is visible from the start.
    let compile_pb = run_compile.then(|| reporter.add_spinner("check"));
    let parallel_pbs: Vec<ProgressBar> = parallel
        .iter()
        .map(|c| reporter.add_spinner(c.label()))
        .collect();
    let coverage_pb = cli
        .opt_in
        .coverage
        .then(|| reporter.add_spinner("coverage"));

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

    // Phase 3: coverage gate (only if --coverage and tests succeeded).
    let coverage_outcome = cli.opt_in.coverage.then(|| {
        let outcome = run_coverage(
            cli,
            &parallel,
            &parallel_outcomes,
            compile_passed,
            has_llvm_cov,
        );
        if let Some(pb) = &coverage_pb {
            reporter.finish_spinner(pb, "coverage", outcome.status);
        }
        outcome
    });

    let failure_count = report_results(
        &reporter,
        cli.verbose,
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

fn run_coverage(
    cli: &Cli,
    parallel: &[Box<dyn Check>],
    outcomes: &[CheckOutcome],
    compile_passed: bool,
    has_llvm_cov: bool,
) -> CheckOutcome {
    if !has_llvm_cov {
        log::warn!("cargo-llvm-cov is not installed, skipping coverage");
        return CheckOutcome::skipped();
    }
    let tests_passed = compile_passed
        && parallel
            .iter()
            .zip(outcomes)
            .find(|(c, _)| c.label() == "test")
            .is_some_and(|(_, o)| o.passed());
    if !tests_passed {
        return CheckOutcome::skipped();
    }
    let threshold = cli.opt_in.min_coverage.to_string();
    checks::run_cargo_outcome("llvm-cov", &["report", "--fail-under-lines", &threshold])
}

fn report_results(
    reporter: &Reporter,
    verbose: u8,
    compile_outcome: Option<&CheckOutcome>,
    parallel: &[Box<dyn Check>],
    parallel_outcomes: &[CheckOutcome],
    compile_passed: bool,
    coverage_outcome: Option<&CheckOutcome>,
) -> usize {
    // PASS sections first when verbose so the operator can scan green-to-red.
    if verbose >= 1 {
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

    // Count failures.
    let mut count = 0;
    if compile_outcome.is_some_and(CheckOutcome::failed) {
        count += 1;
    }
    if compile_passed {
        count += parallel_outcomes.iter().filter(|o| o.failed()).count();
    }
    if coverage_outcome.is_some_and(CheckOutcome::failed) {
        count += 1;
    }
    count
}
