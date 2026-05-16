// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Coverage gate. Parses the JSON summary from `cargo llvm-cov report`
//! and enforces per-metric thresholds.

use serde::Deserialize;

use super::{Check, Runner, combine_streams};
use crate::config::CoverageConfig;
use crate::reporter::{CheckOutcome, TaskStatus};

const COV_REPORT_BRANCH_ARGS: &[&str] = &["report", "--json", "--summary-only", "--branch"];
const COV_REPORT_PLAIN_ARGS: &[&str] = &["report", "--json", "--summary-only"];

pub struct CoverageCheck {
    pub thresholds: CoverageConfig,
    /// Whether to ask `llvm-cov report` for branch coverage and to
    /// enforce the branches threshold. Off on stable Rust; the runner
    /// keys it on [`crate::tooling::is_nightly`].
    pub branch_coverage: bool,
}

impl CoverageCheck {
    pub const LABEL: &'static str = "coverage";

    /// Pick the `llvm-cov report` argv that matches the current
    /// branch-coverage stance. Centralised so `cmd()`, `run()`, and the
    /// `--verbose` banner cannot drift from each other.
    const fn report_args(&self) -> &'static [&'static str] {
        if self.branch_coverage {
            COV_REPORT_BRANCH_ARGS
        } else {
            COV_REPORT_PLAIN_ARGS
        }
    }
}

impl Check for CoverageCheck {
    fn label(&self) -> &'static str {
        Self::LABEL
    }

    fn cmd(&self) -> String {
        format!("cargo llvm-cov {}", self.report_args().join(" "))
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        match collect_report(runner, self.report_args()) {
            Ok(report) => evaluate(&report, self.thresholds, self.branch_coverage),
            Err(output) => CheckOutcome {
                status: TaskStatus::Fail,
                output,
            },
        }
    }

    /// Coverage is never scheduled through [`crate::checks::Plan`] —
    /// the runner forks it off after the chain's `test` slot succeeds —
    /// so `None` here is a non-event. It is still correct on its own
    /// terms: `llvm-cov report` only reads cached profraws and does not
    /// take Cargo's per-`target/` lock.
    fn chain_position(&self) -> Option<u8> {
        None
    }
}

fn collect_report(runner: &dyn Runner, args: &[&str]) -> Result<Report, String> {
    match runner.spawn("llvm-cov", args, &[]) {
        Ok(sr) if sr.success => serde_json::from_slice::<Report>(&sr.stdout)
            .map_err(|e| format!("malformed llvm-cov JSON: {e}")),
        // Some llvm-cov failures write diagnostics to stdout, so both
        // streams must surface to the user.
        Ok(sr) => Err(combine_streams(&sr.stdout, &sr.stderr)),
        Err(e) => Err(format!("failed to launch `cargo llvm-cov`: {e}")),
    }
}

fn evaluate(report: &Report, t: CoverageConfig, branch_coverage: bool) -> CheckOutcome {
    let mut lines: Vec<String> = Vec::new();
    let mut passed = true;

    if report.data.is_empty() {
        return CheckOutcome {
            status: TaskStatus::Fail,
            output: "coverage report contains no data entries".to_string(),
        };
    }

    for entry in &report.data {
        if entry.files.is_empty() {
            lines.push("FAIL no files reported".to_string());
            passed = false;
            continue;
        }
        let mut any_real = false;
        for (name, metric, threshold) in metric_rows(entry, t, branch_coverage) {
            if metric.count == 0 {
                lines.push(format!("ok   {name:<METRIC_NAME_WIDTH$}: 0/0 (vacuous)"));
                continue;
            }
            any_real = true;
            // Integer comparison rather than f64 percentages so the gate
            // is exact at ULP boundaries. The multiplications run in
            // u128 so they cannot overflow for any conceivable
            // count/threshold pair.
            if u128::from(metric.covered) * 100 < u128::from(metric.count) * u128::from(threshold) {
                let missing = metric.count.saturating_sub(metric.covered);
                lines.push(format!(
                    "FAIL {name:<METRIC_NAME_WIDTH$}: {covered}/{total} ({pct}) — threshold {threshold}%, missing {missing}",
                    covered = metric.covered,
                    total = metric.count,
                    pct = format_pct(metric.covered, metric.count),
                ));
                passed = false;
            } else {
                lines.push(format!(
                    "ok   {name:<METRIC_NAME_WIDTH$}: {covered}/{total} ({pct})",
                    covered = metric.covered,
                    total = metric.count,
                    pct = format_pct(metric.covered, metric.count),
                ));
            }
        }
        if !any_real {
            lines.push(
                "FAIL every metric reports count 0 (broken instrumentation or no tests collected)"
                    .to_string(),
            );
            passed = false;
        }
    }

    lines.push(String::new());
    let inspect_cmd = if branch_coverage {
        "Inspect: cargo llvm-cov --branch --html"
    } else {
        "Inspect: cargo llvm-cov --html"
    };
    lines.push(inspect_cmd.to_string());
    lines.push("         target/llvm-cov/html/index.html".to_string());

    CheckOutcome {
        status: if passed {
            TaskStatus::Pass
        } else {
            TaskStatus::Fail
        },
        output: lines.join("\n"),
    }
}

/// Right-pad width applied to metric names so the `count/covered`
/// column lines up. Equal to the longest name (`"functions"`).
const METRIC_NAME_WIDTH: usize = 9;

/// Effective branches threshold when measurement is active. Mirrors the
/// pre-`Option` behaviour: an unset `branches` enforces 100%, just like
/// the other metrics.
const DEFAULT_BRANCH_THRESHOLD: u8 = 100;

/// Materialise the metric rows in display order. The `branches` row is
/// only included when `branch_coverage` is true; on stable Rust we did
/// not pass `--branch`, so `llvm-cov` reports zeros for it and surfacing
/// the row would be misleading.
fn metric_rows(
    entry: &DataEntry,
    t: CoverageConfig,
    branch_coverage: bool,
) -> Vec<(&'static str, Metric, u8)> {
    let mut rows = vec![
        ("functions", entry.totals.functions, t.functions),
        ("lines", entry.totals.lines, t.lines),
        ("regions", entry.totals.regions, t.regions),
    ];
    if branch_coverage {
        rows.push((
            "branches",
            entry.totals.branches,
            t.branches.unwrap_or(DEFAULT_BRANCH_THRESHOLD),
        ));
    }
    rows
}

/// Render `covered/count` as a two-decimal percentage (e.g. `"99.50%"`).
/// Integer arithmetic so the displayed value cannot disagree with the
/// gate. Caller has already excluded `count == 0`.
fn format_pct(covered: u64, count: u64) -> String {
    // Scale by 10_000 to recover two decimal places as integers.
    let scaled = u128::from(covered) * 10_000 / u128::from(count);
    let whole = scaled / 100;
    let frac = scaled % 100;
    format!("{whole}.{frac:02}%")
}

#[derive(Deserialize, Debug)]
pub struct Report {
    data: Vec<DataEntry>,
}

#[derive(Deserialize, Default, Debug)]
struct DataEntry {
    #[serde(default)]
    totals: Metrics,
    #[serde(default)]
    files: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default, Debug)]
struct Metrics {
    #[serde(default)]
    functions: Metric,
    #[serde(default)]
    lines: Metric,
    #[serde(default)]
    regions: Metric,
    #[serde(default)]
    branches: Metric,
}

#[derive(Deserialize, Default, Clone, Copy, Debug)]
struct Metric {
    #[serde(default)]
    count: u64,
    #[serde(default)]
    covered: u64,
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::checks::{FakeRunner, SpawnResult};
    use std::io;

    const COVERED_REPORT: &str = r#"{ "data": [{ "files": [{}], "totals": {
        "functions": { "count": 10, "covered": 10 },
        "lines": { "count": 100, "covered": 100 },
        "regions": { "count": 50, "covered": 50 },
        "branches": { "count": 20, "covered": 20 }
    } }] }"#;

    fn report_from(json: &str) -> Report {
        serde_json::from_str(json).expect("valid json")
    }

    fn fake_with_stdout(stdout: &[u8], success: bool) -> FakeRunner {
        FakeRunner::with_responses(vec![Ok(SpawnResult {
            success,
            stdout: stdout.to_vec(),
            stderr: Vec::new(),
        })])
    }

    #[test]
    fn cmd_runs_cargo_llvm_cov_report_with_branch_flag_when_branch_coverage_on() {
        let c = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: true,
        };
        let cmd = c.cmd();
        assert!(cmd.contains("cargo llvm-cov report"));
        assert!(cmd.contains("--json"));
        assert!(cmd.contains("--summary-only"));
        assert!(cmd.contains("--branch"));
    }

    #[test]
    fn cmd_drops_branch_flag_when_branch_coverage_off() {
        // On stable lockpick must not pass `--branch`, or `llvm-cov`
        // bails with an unhelpful raw rustc error.
        let c = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: false,
        };
        let cmd = c.cmd();
        assert!(cmd.contains("cargo llvm-cov report"));
        assert!(!cmd.contains("--branch"));
    }

    #[test]
    fn label_constant_matches_trait_method() {
        assert_eq!(CoverageCheck::LABEL, "coverage");
        let c = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: true,
        };
        assert_eq!(c.label(), CoverageCheck::LABEL);
    }

    #[test]
    fn evaluate_passes_when_all_metrics_at_100() {
        let report = report_from(COVERED_REPORT);
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.passed(), "got status {:?}", outcome.output);
    }

    #[test]
    fn evaluate_fails_when_branch_below_threshold() {
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 10, "covered": 10 },
                "lines": { "count": 100, "covered": 100 },
                "regions": { "count": 50, "covered": 50 },
                "branches": { "count": 20, "covered": 10 }
            } }] }"#,
        );
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.failed());
        assert!(outcome.output.contains("FAIL branches"));
        assert!(outcome.output.contains("missing 10"));
    }

    #[test]
    fn evaluate_passes_with_relaxed_threshold() {
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 10, "covered": 10 },
                "lines": { "count": 100, "covered": 100 },
                "regions": { "count": 50, "covered": 50 },
                "branches": { "count": 20, "covered": 10 }
            } }] }"#,
        );
        let thresholds = CoverageConfig {
            functions: 100,
            lines: 100,
            regions: 100,
            branches: Some(50),
        };
        let outcome = evaluate(&report, thresholds, true);
        assert!(outcome.passed(), "got: {}", outcome.output);
    }

    #[test]
    fn evaluate_passes_when_metric_sits_exactly_on_threshold() {
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 10, "covered": 10 },
                "lines": { "count": 100, "covered": 100 },
                "regions": { "count": 50, "covered": 50 },
                "branches": { "count": 100, "covered": 50 }
            } }] }"#,
        );
        let thresholds = CoverageConfig {
            functions: 100,
            lines: 100,
            regions: 100,
            branches: Some(50),
        };
        let outcome = evaluate(&report, thresholds, true);
        assert!(outcome.passed(), "got: {}", outcome.output);
    }

    #[test]
    fn evaluate_fails_one_point_below_threshold() {
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 10, "covered": 10 },
                "lines": { "count": 100, "covered": 100 },
                "regions": { "count": 50, "covered": 50 },
                "branches": { "count": 100, "covered": 49 }
            } }] }"#,
        );
        let thresholds = CoverageConfig {
            functions: 100,
            lines: 100,
            regions: 100,
            branches: Some(50),
        };
        let outcome = evaluate(&report, thresholds, true);
        assert!(outcome.failed());
        assert!(outcome.output.contains("FAIL branches"));
    }

    #[test]
    fn evaluate_skips_branches_row_when_branch_coverage_off() {
        // Stable run: `--branch` was never passed, so even if the report
        // happens to contain a `branches` block it would be all zeros.
        // The row must not surface at all, neither as a real gate nor
        // as a `0/0 (vacuous)` cell that would suggest measurement.
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 10, "covered": 10 },
                "lines": { "count": 100, "covered": 100 },
                "regions": { "count": 50, "covered": 50 },
                "branches": { "count": 0, "covered": 0 }
            } }] }"#,
        );
        let outcome = evaluate(&report, CoverageConfig::default(), false);
        assert!(outcome.passed(), "got: {}", outcome.output);
        assert!(
            !outcome.output.contains("branches"),
            "branches row leaked into stable output:\n{}",
            outcome.output
        );
    }

    #[test]
    fn evaluate_inspect_hint_matches_branch_coverage_setting() {
        // The inspect hint we hand back to the user must mirror what we
        // actually ran, or copy-pasting it reproduces the bug we fix.
        let report = report_from(COVERED_REPORT);
        let on = evaluate(&report, CoverageConfig::default(), true);
        assert!(on.output.contains("cargo llvm-cov --branch --html"));

        let off = evaluate(&report, CoverageConfig::default(), false);
        assert!(off.output.contains("cargo llvm-cov --html"));
        assert!(!off.output.contains("--branch"));
    }

    #[test]
    fn evaluate_with_branches_unset_defaults_to_full_branch_threshold() {
        // None means "user did not set this"; on nightly the gate must
        // still demand 100% coverage to match the always-on semantics
        // of the other three metrics.
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 10, "covered": 10 },
                "lines": { "count": 100, "covered": 100 },
                "regions": { "count": 50, "covered": 50 },
                "branches": { "count": 20, "covered": 19 }
            } }] }"#,
        );
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.failed());
        assert!(outcome.output.contains("FAIL branches"));
        assert!(outcome.output.contains("threshold 100%"));
    }

    #[test]
    fn format_pct_renders_two_decimals_for_non_round_ratios() {
        assert_eq!(format_pct(1, 3), "33.33%");
        assert_eq!(format_pct(2, 3), "66.66%");
        assert_eq!(format_pct(1, 8), "12.50%");
        assert_eq!(format_pct(0, 100), "0.00%");
        assert_eq!(format_pct(50, 50), "100.00%");
    }

    #[test]
    fn evaluate_treats_zero_count_as_vacuous() {
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 10, "covered": 10 },
                "lines": { "count": 100, "covered": 100 },
                "regions": { "count": 50, "covered": 50 },
                "branches": { "count": 0, "covered": 0 }
            } }] }"#,
        );
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.passed());
        assert!(outcome.output.contains("0/0 (vacuous)"));
    }

    #[test]
    fn evaluate_rejects_report_with_no_data_entries() {
        let report = report_from(r#"{ "data": [] }"#);
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.failed());
        assert!(outcome.output.contains("no data entries"));
    }

    #[test]
    fn evaluate_rejects_entries_with_no_files() {
        let report = report_from(
            r#"{ "data": [{ "files": [], "totals": {
                "functions": { "count": 1, "covered": 1 }
            } }] }"#,
        );
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.failed());
        assert!(outcome.output.contains("no files reported"));
    }

    #[test]
    fn evaluate_rejects_all_zero_metrics_as_broken_instrumentation() {
        let report = report_from(
            r#"{ "data": [{ "files": [{}], "totals": {
                "functions": { "count": 0, "covered": 0 },
                "lines": { "count": 0, "covered": 0 },
                "regions": { "count": 0, "covered": 0 },
                "branches": { "count": 0, "covered": 0 }
            } }] }"#,
        );
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.failed());
        assert!(outcome.output.contains("broken instrumentation"));
    }

    #[test]
    fn collect_report_parses_runner_stdout_on_success() {
        let fake = fake_with_stdout(COVERED_REPORT.as_bytes(), true);
        let report = collect_report(&fake, COV_REPORT_BRANCH_ARGS).expect("parsed");
        let outcome = evaluate(&report, CoverageConfig::default(), true);
        assert!(outcome.passed());
    }

    #[test]
    fn collect_report_surfaces_stderr_on_non_zero_status() {
        let fake = FakeRunner::with_responses(vec![Ok(SpawnResult {
            success: false,
            stdout: Vec::new(),
            stderr: b"llvm-cov boom".to_vec(),
        })]);
        let err = collect_report(&fake, COV_REPORT_BRANCH_ARGS).unwrap_err();
        assert!(err.contains("llvm-cov boom"));
    }

    #[test]
    fn collect_report_includes_stdout_in_failure_message() {
        let fake = FakeRunner::with_responses(vec![Ok(SpawnResult {
            success: false,
            stdout: b"error on stdout".to_vec(),
            stderr: b"err on stderr".to_vec(),
        })]);
        let err = collect_report(&fake, COV_REPORT_BRANCH_ARGS).unwrap_err();
        assert!(err.contains("error on stdout"), "got: {err}");
        assert!(err.contains("err on stderr"), "got: {err}");
    }

    #[test]
    fn collect_report_complains_about_malformed_json() {
        let fake = fake_with_stdout(b"definitely not json", true);
        let err = collect_report(&fake, COV_REPORT_BRANCH_ARGS).unwrap_err();
        assert!(err.contains("malformed llvm-cov JSON"));
    }

    #[test]
    fn collect_report_surfaces_io_error_with_launch_message() {
        let fake = FakeRunner::with_responses(vec![Err(io::Error::other("ENOENT"))]);
        let err = collect_report(&fake, COV_REPORT_BRANCH_ARGS).unwrap_err();
        assert!(err.contains("failed to launch"));
        assert!(err.contains("ENOENT"));
    }

    #[test]
    fn run_passes_when_collect_report_succeeds_and_thresholds_met() {
        let fake = fake_with_stdout(COVERED_REPORT.as_bytes(), true);
        let check = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: true,
        };
        let outcome = check.run(&fake);
        assert!(outcome.passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "llvm-cov");
        assert!(calls[0].args.contains(&"report".to_string()));
        assert!(
            calls[0].args.iter().any(|a| a == "--branch"),
            "expected `--branch` in args when branch_coverage = true, got: {:?}",
            calls[0].args
        );
    }

    #[test]
    fn run_without_branch_coverage_omits_branch_flag_from_spawn_args() {
        let fake = fake_with_stdout(COVERED_REPORT.as_bytes(), true);
        let check = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: false,
        };
        let _ = check.run(&fake);
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "llvm-cov");
        assert!(calls[0].args.contains(&"report".to_string()));
        assert!(
            !calls[0].args.iter().any(|a| a == "--branch"),
            "stable run must not request branch coverage, got: {:?}",
            calls[0].args
        );
    }

    #[test]
    fn run_returns_fail_when_collect_report_errors() {
        let fake = fake_with_stdout(b"definitely not json", true);
        let check = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: true,
        };
        let outcome = check.run(&fake);
        assert!(outcome.failed());
        assert!(outcome.output.contains("malformed llvm-cov JSON"));
    }

    #[test]
    fn chain_position_is_none_because_coverage_is_runner_scheduled() {
        let check = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: true,
        };
        assert_eq!(check.chain_position(), None);
    }
}
