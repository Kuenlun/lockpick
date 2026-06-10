// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
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
    /// enforce the branches threshold. Off on stable Rust. The runner
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

    /// Coverage is never scheduled through [`crate::checks::Plan`].
    /// The runner forks it off after the chain's `test` slot passes.
    /// `None` is also correct on its own terms: `llvm-cov report` only
    /// reads cached profraws and does not take `target/.cargo-lock`.
    fn chain_position(&self) -> Option<u8> {
        None
    }
}

fn collect_report(runner: &dyn Runner, args: &[&str]) -> Result<Report, String> {
    match runner.spawn("llvm-cov", args, &[]) {
        Ok(sr) if sr.success => serde_json::from_slice::<Report>(&sr.stdout)
            .map_err(|e| format!("malformed llvm-cov JSON: {e}")),
        // llvm-cov sometimes writes diagnostics to stdout, so surface
        // both streams.
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
            // Integer comparison rather than f64 percentages so the
            // gate is exact at ULP boundaries. u128 cannot overflow
            // for any conceivable count/threshold pair.
            if u128::from(metric.covered) * 100 < u128::from(metric.count) * u128::from(threshold) {
                let missing = metric.count.saturating_sub(metric.covered);
                lines.push(format!(
                    "FAIL {name:<METRIC_NAME_WIDTH$}: {covered}/{total} ({pct}), threshold {threshold}%, missing {missing}",
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

/// Metric rows in display order. The `branches` row is dropped on
/// stable: without `--branch`, llvm-cov reports zeros and the row
/// would mislead.
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
    use serde_json::json;

    use super::*;

    fn metric(covered: u64, count: u64) -> serde_json::Value {
        json!({ "count": count, "covered": covered })
    }

    fn totals(covered: u64, count: u64) -> serde_json::Value {
        json!({
            "functions": metric(covered, count),
            "lines": metric(covered, count),
            "regions": metric(covered, count),
            "branches": metric(covered, count),
        })
    }

    fn report(totals: &serde_json::Value) -> Report {
        serde_json::from_value(json!({
            "data": [{ "totals": totals, "files": [{}] }],
        }))
        .unwrap()
    }

    #[test]
    fn full_coverage_passes_default_thresholds() {
        let outcome = evaluate(&report(&totals(10, 10)), CoverageConfig::default(), true);
        assert!(outcome.passed(), "output:\n{}", outcome.output);
    }

    #[test]
    fn threshold_boundary_is_exact_integer_math() {
        // 99/100 sits exactly on a 99% threshold and must pass; one
        // fewer covered line must fail. No float rounding involved.
        let thresholds = CoverageConfig {
            functions: 99,
            lines: 99,
            regions: 99,
            branches: None,
        };
        assert!(evaluate(&report(&totals(99, 100)), thresholds, false).passed());

        let failing = evaluate(&report(&totals(98, 100)), thresholds, false);
        assert!(failing.failed());
        assert!(
            failing.output.contains("threshold 99%") && failing.output.contains("missing 2"),
            "output:\n{}",
            failing.output
        );
    }

    #[test]
    fn unset_branches_threshold_enforces_100_on_nightly() {
        let mixed = json!({
            "functions": metric(10, 10),
            "lines": metric(10, 10),
            "regions": metric(10, 10),
            "branches": metric(9, 10),
        });
        let outcome = evaluate(&report(&mixed), CoverageConfig::default(), true);
        assert!(outcome.failed());
        assert!(
            outcome.output.contains("FAIL branches"),
            "output:\n{}",
            outcome.output
        );
    }

    #[test]
    fn branches_row_and_inspect_hint_follow_branch_coverage() {
        let with = evaluate(&report(&totals(10, 10)), CoverageConfig::default(), true);
        assert!(with.output.contains("branches"));
        assert!(with.output.contains("--branch --html"));

        let without = evaluate(&report(&totals(10, 10)), CoverageConfig::default(), false);
        assert!(!without.output.contains("branches"));
        assert!(!without.output.contains("--branch"));
    }

    #[test]
    fn vacuous_metrics_pass_individually_but_not_collectively() {
        let all_zero = evaluate(&report(&totals(0, 0)), CoverageConfig::default(), true);
        assert!(all_zero.failed());
        assert!(
            all_zero.output.contains("broken instrumentation"),
            "output:\n{}",
            all_zero.output
        );

        let one_vacuous = json!({
            "functions": metric(0, 0),
            "lines": metric(10, 10),
            "regions": metric(10, 10),
            "branches": metric(10, 10),
        });
        let outcome = evaluate(&report(&one_vacuous), CoverageConfig::default(), true);
        assert!(outcome.passed(), "output:\n{}", outcome.output);
        assert!(outcome.output.contains("vacuous"));
    }

    #[test]
    fn empty_report_data_and_missing_files_fail() {
        let empty: Report = serde_json::from_value(json!({ "data": [] })).unwrap();
        let outcome = evaluate(&empty, CoverageConfig::default(), false);
        assert!(outcome.failed());
        assert!(outcome.output.contains("no data entries"));

        let no_files: Report = serde_json::from_value(json!({
            "data": [{ "totals": totals(10, 10), "files": [] }],
        }))
        .unwrap();
        let outcome = evaluate(&no_files, CoverageConfig::default(), false);
        assert!(outcome.failed());
        assert!(outcome.output.contains("no files reported"));
    }

    #[test]
    fn format_pct_truncates_to_two_decimals() {
        assert_eq!(format_pct(199, 200), "99.50%");
        assert_eq!(format_pct(1, 3), "33.33%");
        assert_eq!(format_pct(10, 10), "100.00%");
    }

    #[test]
    fn cmd_matches_the_branch_coverage_stance() {
        let on = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: true,
        };
        assert_eq!(
            on.cmd(),
            "cargo llvm-cov report --json --summary-only --branch"
        );
        let off = CoverageCheck {
            thresholds: CoverageConfig::default(),
            branch_coverage: false,
        };
        assert_eq!(off.cmd(), "cargo llvm-cov report --json --summary-only");
    }
}
