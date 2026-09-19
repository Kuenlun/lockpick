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

pub(crate) struct CoverageCheck {
    pub(crate) options: super::util::BuildOptions,
    pub(crate) thresholds: CoverageConfig,
    /// Whether to ask `llvm-cov report` for branch coverage and to
    /// enforce the branches threshold. Off on stable Rust. The runner
    /// keys it on [`crate::tooling::is_nightly`].
    pub(crate) branch_coverage: bool,
}

impl CoverageCheck {
    pub(crate) const LABEL: &'static str = "coverage";

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
        format!(
            "cargo llvm-cov {}",
            self.options.args(self.report_args()).join(" ")
        )
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        match collect_report(runner, &self.options.args(self.report_args())) {
            Ok(report) => evaluate(&report, self.thresholds, self.branch_coverage),
            Err(output) => CheckOutcome {
                status: TaskStatus::Fail,
                output,
            },
        }
    }
}

fn collect_report(runner: &dyn Runner, args: &[&str]) -> Result<Report, String> {
    match runner.spawn("llvm-cov", args, &[]) {
        Ok(sr) if sr.success => parse_report(&sr.stdout).map_err(|e| {
            format!("{e}\n{}", String::from_utf8_lossy(&sr.stderr))
                .trim()
                .to_owned()
        }),
        // llvm-cov sometimes writes diagnostics to stdout, so surface
        // both streams.
        Ok(sr) => Err(combine_streams(&sr.stdout, &sr.stderr)),
        Err(e) => Err(format!("failed to launch `cargo llvm-cov`: {e}")),
    }
}

fn parse_report(bytes: &[u8]) -> Result<Report, String> {
    let report: Report =
        serde_json::from_slice(bytes).map_err(|e| format!("malformed llvm-cov JSON: {e}"))?;
    if report.kind != "llvm.coverage.json.export"
        || !matches!(report.version.split('.').next(), Some("2" | "3"))
    {
        return Err(
            "unsupported llvm-cov JSON type or version (expected export version 2 or 3)".into(),
        );
    }
    Ok(report)
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
        if entry.files.iter().any(|file| file.filename.is_empty()) {
            lines.push("FAIL empty source filename in coverage report".to_string());
            passed = false;
        }
        if branch_coverage && entry.totals.branches.is_none() {
            lines.push("FAIL missing branches metric in coverage report".to_string());
            passed = false;
        }
        let mut any_real = false;
        for (name, metric, threshold) in metric_rows(entry, t, branch_coverage) {
            if metric.covered > metric.count {
                lines.push(format!("FAIL {name}: covered count exceeds total count"));
                passed = false;
                continue;
            }
            let Some(count) = std::num::NonZeroU64::new(metric.count) else {
                lines.push(format!("ok   {name:<METRIC_NAME_WIDTH$}: 0/0 (vacuous)"));
                continue;
            };
            any_real = true;
            // Integer comparison rather than f64 percentages so the
            // gate is exact at ULP boundaries. u128 cannot overflow
            // for any conceivable count/threshold pair.
            if u128::from(metric.covered).saturating_mul(100)
                < u128::from(metric.count).saturating_mul(u128::from(threshold))
            {
                let missing = metric.count.saturating_sub(metric.covered);
                lines.push(format!(
                    "FAIL {name:<METRIC_NAME_WIDTH$}: {covered}/{total} ({pct}), threshold {threshold}%, missing {missing}",
                    covered = metric.covered,
                    total = metric.count,
                    pct = format_pct(metric.covered, count),
                ));
                passed = false;
            } else {
                lines.push(format!(
                    "ok   {name:<METRIC_NAME_WIDTH$}: {covered}/{total} ({pct})",
                    covered = metric.covered,
                    total = metric.count,
                    pct = format_pct(metric.covered, count),
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
            entry.totals.branches.unwrap_or_default(),
            t.branches.unwrap_or(DEFAULT_BRANCH_THRESHOLD),
        ));
    }
    rows
}

/// Render `covered/count` as a two-decimal percentage (e.g. `"99.50%"`).
/// Integer arithmetic so the displayed value cannot disagree with the
/// gate. Caller has already excluded `count == 0`.
fn format_pct(covered: u64, count: std::num::NonZeroU64) -> String {
    // Scale by 10_000 to recover two decimal places as integers.
    let scaled = u128::from(covered).saturating_mul(10_000) / std::num::NonZeroU128::from(count);
    let whole = scaled / 100;
    let frac = scaled % 100;
    format!("{whole}.{frac:02}%")
}

#[derive(Deserialize, Debug)]
pub(crate) struct Report {
    #[serde(rename = "type")]
    kind: String,
    version: String,
    data: Vec<DataEntry>,
}

#[derive(Deserialize, Default, Debug)]
struct DataEntry {
    totals: Metrics,
    files: Vec<SourceFile>,
}

#[derive(Deserialize, Default, Debug)]
struct SourceFile {
    filename: String,
}

#[derive(Deserialize, Default, Debug)]
struct Metrics {
    functions: Metric,
    lines: Metric,
    regions: Metric,
    branches: Option<Metric>,
}

#[derive(Deserialize, Default, Clone, Copy, Debug)]
struct Metric {
    count: u64,
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
            "type": "llvm.coverage.json.export", "version": "2.0.1",
            "data": [{ "totals": totals, "files": [{ "filename": "src/lib.rs" }] }],
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
        let empty: Report = serde_json::from_value(
            json!({ "type": "llvm.coverage.json.export", "version": "2.0.1", "data": [] }),
        )
        .unwrap();
        let outcome = evaluate(&empty, CoverageConfig::default(), false);
        assert!(outcome.failed());
        assert!(outcome.output.contains("no data entries"));

        let no_files: Report = serde_json::from_value(json!({
            "type": "llvm.coverage.json.export", "version": "2.0.1",
            "data": [{ "totals": totals(10, 10), "files": [] }],
        }))
        .unwrap();
        let outcome = evaluate(&no_files, CoverageConfig::default(), false);
        assert!(outcome.failed());
        assert!(outcome.output.contains("no files reported"));
    }

    #[test]
    fn format_pct_truncates_to_two_decimals() {
        let count = |value| std::num::NonZeroU64::new(value).unwrap();
        assert_eq!(format_pct(199, count(200)), "99.50%");
        assert_eq!(format_pct(1, count(3)), "33.33%");
        assert_eq!(format_pct(10, count(10)), "100.00%");
    }

    #[test]
    fn cmd_matches_the_branch_coverage_stance() {
        let on = CoverageCheck {
            options: super::super::util::BuildOptions::default(),
            thresholds: CoverageConfig::default(),
            branch_coverage: true,
        };
        assert_eq!(
            on.cmd(),
            "cargo llvm-cov report --json --summary-only --branch"
        );
        let off = CoverageCheck {
            options: super::super::util::BuildOptions::default(),
            thresholds: CoverageConfig::default(),
            branch_coverage: false,
        };
        assert_eq!(off.cmd(), "cargo llvm-cov report --json --summary-only");
    }
    #[test]
    fn report_schema_rejects_missing_and_invalid_counts() {
        let valid = json!({
            "type": "llvm.coverage.json.export", "version": "2.0.1",
            "data": [{ "totals": totals(1, 1), "files": [{ "filename": "src/lib.rs" }] }],
        });
        assert!(parse_report(&serde_json::to_vec(&valid).unwrap()).is_ok());
        for pointer in [
            "/type",
            "/version",
            "/data/0/totals/functions",
            "/data/0/totals/lines/count",
            "/data/0/totals/regions/covered",
            "/data/0/files/0/filename",
        ] {
            let mut broken = valid.clone();
            *broken.pointer_mut(pointer).unwrap() = json!(null);
            assert!(
                parse_report(&serde_json::to_vec(&broken).unwrap()).is_err(),
                "{pointer}"
            );
        }
        for version in ["", "1.0", "20.0"] {
            let mut broken = valid.clone();
            *broken.get_mut("version").unwrap() = json!(version);
            assert!(parse_report(&serde_json::to_vec(&broken).unwrap()).is_err());
        }
        assert!(parse_report(b"not JSON").is_err());
        for (covered, count) in [(1, 0), (11, 10), (u64::MAX, 1)] {
            assert!(
                evaluate(
                    &report(&totals(covered, count)),
                    CoverageConfig::default(),
                    true
                )
                .failed()
            );
        }
        let mut missing = totals(10, 10);
        assert!(
            missing
                .as_object_mut()
                .unwrap()
                .remove("branches")
                .is_some()
        );
        assert!(evaluate(&report(&missing), CoverageConfig::default(), true).failed());
        assert!(evaluate(&report(&missing), CoverageConfig::default(), false).passed());
    }

    #[test]
    fn large_counts_and_zero_thresholds_do_not_overflow_or_round_up() {
        assert!(
            evaluate(
                &report(&totals(u64::MAX, u64::MAX)),
                CoverageConfig::default(),
                true
            )
            .passed()
        );
        assert!(
            evaluate(
                &report(&totals(u64::MAX - 1, u64::MAX)),
                CoverageConfig::default(),
                true
            )
            .failed()
        );
        let thresholds = CoverageConfig {
            functions: 0,
            lines: 0,
            regions: 0,
            branches: Some(0),
        };
        assert!(evaluate(&report(&totals(0, 1)), thresholds, true).passed());
        assert!(evaluate(&report(&totals(0, 0)), thresholds, true).failed());
    }

    struct ReportRunner {
        result: std::io::Result<super::super::runner::SpawnResult>,
    }

    impl Runner for ReportRunner {
        fn spawn(
            &self,
            sub: &str,
            args: &[&str],
            envs: &[(&str, &str)],
        ) -> std::io::Result<super::super::runner::SpawnResult> {
            assert_eq!(sub, "llvm-cov");
            assert_eq!(args, COV_REPORT_PLAIN_ARGS);
            assert_eq!(envs, []);
            self.result
                .as_ref()
                .cloned()
                .map_err(|e| std::io::Error::new(e.kind(), e.to_string()))
        }
    }

    #[test]
    fn report_execution_preserves_diagnostics_and_launch_errors() {
        use super::super::runner::SpawnResult;
        for (result, message) in [
            (Err(std::io::Error::other("launch denied")), "launch denied"),
            (
                Ok(SpawnResult {
                    success: false,
                    stdout: b"report failed".to_vec(),
                    stderr: b"merge failed".to_vec(),
                }),
                "merge failed",
            ),
            (
                Ok(SpawnResult {
                    success: true,
                    stdout: b"{}".to_vec(),
                    stderr: b"tool diagnostic".to_vec(),
                }),
                "tool diagnostic",
            ),
        ] {
            let check = CoverageCheck {
                options: super::super::util::BuildOptions::default(),
                thresholds: CoverageConfig::default(),
                branch_coverage: false,
            };
            let outcome = check.run(&ReportRunner { result });
            assert!(outcome.failed());
            assert!(outcome.output.contains(message), "{}", outcome.output);
        }
    }
    #[test]
    fn source_filenames_and_export_types_are_required() {
        let mut value = json!({
            "type": "llvm.coverage.json.export", "version": "3.1.0",
            "data": [{ "totals": totals(1, 1), "files": [{ "filename": "" }] }],
        });
        let parsed = parse_report(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(evaluate(&parsed, CoverageConfig::default(), false).failed());
        *value.get_mut("type").unwrap() = json!("another-report");
        assert!(parse_report(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
