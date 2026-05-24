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
