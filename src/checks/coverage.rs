// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Coverage gate. Parses the JSON summary produced by `cargo llvm-cov
//! report` and enforces per-metric thresholds (functions, lines, regions,
//! branches). The report is generated from `.profraw` files emitted by
//! the previous `test` check when it ran with instrumentation.

use std::process::{Command, Stdio};

use serde::Deserialize;

use super::Check;
use crate::config::CoverageConfig;
use crate::reporter::{CheckOutcome, TaskStatus};

const COV_REPORT_ARGS: &[&str] = &["report", "--json", "--summary-only", "--branch"];

pub struct CoverageCheck {
    pub thresholds: CoverageConfig,
}

impl Check for CoverageCheck {
    fn label(&self) -> &'static str {
        "coverage"
    }

    fn cmd(&self) -> String {
        format!("cargo llvm-cov {}", COV_REPORT_ARGS.join(" "))
    }

    fn run(&self) -> CheckOutcome {
        match collect_report() {
            Ok(report) => evaluate(&report, self.thresholds),
            Err(output) => CheckOutcome {
                status: TaskStatus::Fail,
                output,
            },
        }
    }
}

fn collect_report() -> Result<Report, String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("llvm-cov").args(COV_REPORT_ARGS);
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to launch `cargo llvm-cov`: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    serde_json::from_slice::<Report>(&out.stdout)
        .map_err(|e| format!("malformed llvm-cov JSON: {e}"))
}

// Coverage counts are not large enough in practice for the u64→f64 cast
// to drop meaningful precision; the percentage only needs ~6 significant
// digits anyway.
#[allow(clippy::cast_precision_loss)]
fn evaluate(report: &Report, t: CoverageConfig) -> CheckOutcome {
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
        for (name, metric, threshold) in metric_rows(entry, t) {
            if metric.count == 0 {
                lines.push(format!("ok   {name}: 0/0 (vacuous)"));
                continue;
            }
            any_real = true;
            let pct = (metric.covered as f64) * 100.0 / (metric.count as f64);
            let target = f64::from(threshold);
            if pct + f64::EPSILON < target {
                let missing = metric.count - metric.covered;
                lines.push(format!(
                    "FAIL {name}: {covered}/{total} ({pct:.2}%) — threshold {threshold}%, missing {missing}",
                    covered = metric.covered,
                    total = metric.count,
                ));
                passed = false;
            } else {
                lines.push(format!(
                    "ok   {name}: {covered}/{total} ({pct:.2}%)",
                    covered = metric.covered,
                    total = metric.count,
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
    lines.push("Inspect: cargo llvm-cov --branch --html".to_string());
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

const fn metric_rows(entry: &DataEntry, t: CoverageConfig) -> [(&'static str, Metric, u8); 4] {
    [
        ("functions", entry.totals.functions, t.functions),
        ("lines    ", entry.totals.lines, t.lines),
        ("regions  ", entry.totals.regions, t.regions),
        ("branches ", entry.totals.branches, t.branches),
    ]
}

#[derive(Deserialize)]
struct Report {
    data: Vec<DataEntry>,
}

#[derive(Deserialize, Default)]
struct DataEntry {
    #[serde(default)]
    totals: Metrics,
    #[serde(default)]
    files: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default)]
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

#[derive(Deserialize, Default, Clone, Copy)]
struct Metric {
    #[serde(default)]
    count: u64,
    #[serde(default)]
    covered: u64,
}
