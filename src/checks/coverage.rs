// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Coverage gate. Parses the JSON summary produced by `cargo llvm-cov
//! report` and enforces per-metric thresholds (functions, lines, regions,
//! branches). The report is generated from `.profraw` files emitted by
//! the previous `test` check when it ran with instrumentation.

use serde::Deserialize;

use super::{Check, Runner};
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

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        match collect_report(runner) {
            Ok(report) => evaluate(&report, self.thresholds),
            Err(output) => CheckOutcome {
                status: TaskStatus::Fail,
                output,
            },
        }
    }
}

fn collect_report(runner: &dyn Runner) -> Result<Report, String> {
    match runner.spawn("llvm-cov", COV_REPORT_ARGS, &[]) {
        Ok(sr) if sr.success => serde_json::from_slice::<Report>(&sr.stdout)
            .map_err(|e| format!("malformed llvm-cov JSON: {e}")),
        Ok(sr) => Err(String::from_utf8_lossy(&sr.stderr).into_owned()),
        Err(e) => Err(format!("failed to launch `cargo llvm-cov`: {e}")),
    }
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
    fn cmd_runs_cargo_llvm_cov_report() {
        let c = CoverageCheck {
            thresholds: CoverageConfig::default(),
        };
        let cmd = c.cmd();
        assert!(cmd.contains("cargo llvm-cov report"));
        assert!(cmd.contains("--json"));
        assert!(cmd.contains("--summary-only"));
        assert!(cmd.contains("--branch"));
    }

    #[test]
    fn evaluate_passes_when_all_metrics_at_100() {
        let report = report_from(COVERED_REPORT);
        let outcome = evaluate(&report, CoverageConfig::default());
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
        let outcome = evaluate(&report, CoverageConfig::default());
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
            branches: 50,
        };
        let outcome = evaluate(&report, thresholds);
        assert!(outcome.passed(), "got: {}", outcome.output);
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
        let outcome = evaluate(&report, CoverageConfig::default());
        assert!(outcome.passed());
        assert!(outcome.output.contains("0/0 (vacuous)"));
    }

    #[test]
    fn evaluate_rejects_report_with_no_data_entries() {
        let report = report_from(r#"{ "data": [] }"#);
        let outcome = evaluate(&report, CoverageConfig::default());
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
        let outcome = evaluate(&report, CoverageConfig::default());
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
        let outcome = evaluate(&report, CoverageConfig::default());
        assert!(outcome.failed());
        assert!(outcome.output.contains("broken instrumentation"));
    }

    #[test]
    fn collect_report_parses_runner_stdout_on_success() {
        let fake = fake_with_stdout(COVERED_REPORT.as_bytes(), true);
        let report = collect_report(&fake).expect("parsed");
        let outcome = evaluate(&report, CoverageConfig::default());
        assert!(outcome.passed());
    }

    #[test]
    fn collect_report_surfaces_stderr_on_non_zero_status() {
        let fake = FakeRunner::with_responses(vec![Ok(SpawnResult {
            success: false,
            stdout: Vec::new(),
            stderr: b"llvm-cov boom".to_vec(),
        })]);
        let err = collect_report(&fake).unwrap_err();
        assert!(err.contains("llvm-cov boom"));
    }

    #[test]
    fn collect_report_complains_about_malformed_json() {
        let fake = fake_with_stdout(b"definitely not json", true);
        let err = collect_report(&fake).unwrap_err();
        assert!(err.contains("malformed llvm-cov JSON"));
    }

    #[test]
    fn collect_report_surfaces_io_error_with_launch_message() {
        let fake = FakeRunner::with_responses(vec![Err(io::Error::other("ENOENT"))]);
        let err = collect_report(&fake).unwrap_err();
        assert!(err.contains("failed to launch"));
        assert!(err.contains("ENOENT"));
    }

    #[test]
    fn run_passes_when_collect_report_succeeds_and_thresholds_met() {
        let fake = fake_with_stdout(COVERED_REPORT.as_bytes(), true);
        let check = CoverageCheck {
            thresholds: CoverageConfig::default(),
        };
        let outcome = check.run(&fake);
        assert!(outcome.passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "llvm-cov");
        assert!(calls[0].args.contains(&"report".to_string()));
    }

    #[test]
    fn run_returns_fail_when_collect_report_errors() {
        let fake = fake_with_stdout(b"definitely not json", true);
        let check = CoverageCheck {
            thresholds: CoverageConfig::default(),
        };
        let outcome = check.run(&fake);
        assert!(outcome.failed());
        assert!(outcome.output.contains("malformed llvm-cov JSON"));
    }
}
