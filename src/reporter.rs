// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::io::IsTerminal;
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskStatus {
    Pass,
    Fail,
    Skip,
}

pub(crate) struct CheckOutcome {
    pub(crate) status: TaskStatus,
    pub(crate) output: String,
}

impl CheckOutcome {
    #[must_use]
    pub(crate) const fn passed(&self) -> bool {
        matches!(self.status, TaskStatus::Pass)
    }

    #[must_use]
    pub(crate) const fn failed(&self) -> bool {
        matches!(self.status, TaskStatus::Fail)
    }

    #[must_use]
    pub(crate) const fn skipped() -> Self {
        Self {
            status: TaskStatus::Skip,
            output: String::new(),
        }
    }
}

pub(crate) struct Reporter {
    mp: MultiProgress,
    spin_style: ProgressStyle,
    done_style: ProgressStyle,
    /// Stderr is a TTY: drives spinner rendering and routes `diag`
    /// writes through `MultiProgress` so they interleave cleanly.
    is_tty: bool,
    /// Stdout is a TTY. When false, the report stream is captured
    /// (file, pipe, CI), so the spinner keeps a visible final state on
    /// stderr instead of clearing.
    stdout_is_tty: bool,
    pub(crate) is_verbose: bool,
}

/// Column width used to align check labels in spinners and status lines.
/// Must accommodate the longest concrete `Check::label()`.
pub(crate) const LABEL_WIDTH: usize = 10;

const DONE_TEMPLATE: &str = "  {msg}";
const TICK_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

fn spin_template() -> String {
    format!("  {{msg:<{LABEL_WIDTH}}} {{spinner:.cyan}}")
}

/// Parse an indicatif template, falling back to the default spinner.
/// Keeps the caller infallible under `clippy::expect_used`.
fn parse_template(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template).unwrap_or_else(|_| ProgressStyle::default_spinner())
}

impl Reporter {
    /// Build a [`Reporter`] with the TTY state of stdout and stderr
    /// probed from the process's own streams.
    #[must_use]
    pub(crate) fn auto(is_verbose: bool) -> Self {
        Self::new(
            is_verbose,
            std::io::stderr().is_terminal(),
            std::io::stdout().is_terminal(),
        )
    }

    fn new(is_verbose: bool, is_tty: bool, stdout_is_tty: bool) -> Self {
        let spin_style = parse_template(&spin_template()).tick_chars(TICK_CHARS);
        let done_style = parse_template(DONE_TEMPLATE);
        let mp = if is_tty {
            MultiProgress::new()
        } else {
            MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
        };
        Self {
            mp,
            spin_style,
            done_style,
            is_tty,
            stdout_is_tty,
            is_verbose,
        }
    }

    pub(crate) fn add_spinner(&self, label: &str) -> ProgressBar {
        let pb = self.mp.add(ProgressBar::new_spinner());
        pb.set_style(self.spin_style.clone());
        pb.set_message(label.to_string());
        if self.is_tty {
            pb.enable_steady_tick(Duration::from_millis(80));
        }
        pb
    }

    /// Finish a spinner and emit the matching status line. Routing
    /// depends on which streams are TTYs:
    ///
    /// * stderr TTY + stdout TTY: anchor the spinner in place so
    ///   siblings do not shift up. Skip `reportln` (would duplicate
    ///   the anchored line in the same terminal).
    /// * stderr TTY + stdout captured: anchor on stderr, also emit on
    ///   stdout for the capture.
    /// * stderr non-TTY: spinner is hidden, only emit on stdout.
    pub(crate) fn finish_spinner(&self, pb: &ProgressBar, label: &str, status: TaskStatus) {
        let tag = match status {
            TaskStatus::Pass => "PASS".green().bold(),
            TaskStatus::Fail => "FAIL".red().bold(),
            TaskStatus::Skip => "SKIP".yellow().bold(),
        };
        if self.is_tty {
            pb.set_style(self.done_style.clone());
            pb.finish_with_message(format!("{label:<LABEL_WIDTH$} {tag}"));
        } else {
            pb.finish_and_clear();
        }
        if !(self.is_tty && self.stdout_is_tty) {
            self.reportln(format!("  {label:<LABEL_WIDTH$} {tag}"));
        }
    }

    /// Write a line to the diagnostic stream (stderr): banners, notes,
    /// progress chatter. Routed through `MultiProgress` so it interleaves
    /// cleanly with active spinners in TTY mode.
    #[expect(clippy::print_stderr, reason = "Reporter owns the diagnostic stream.")]
    pub(crate) fn diagln(&self, msg: impl AsRef<str>) {
        if self.is_tty {
            let _write_result = self.mp.println(msg);
        } else {
            eprintln!("{}", msg.as_ref());
        }
    }

    /// Write a line to the report stream (stdout): status lines and the
    /// final summary. `MultiProgress::suspend` pauses spinner drawing
    /// for the write so the two streams do not stomp on each other.
    /// Multi-line blocks should batch under one `suspend` (see
    /// [`Self::print_section`]) to avoid one redraw cycle per line.
    #[expect(clippy::print_stdout, reason = "Reporter owns the report stream.")]
    pub(crate) fn reportln(&self, msg: impl AsRef<str>) {
        self.mp.suspend(|| println!("{}", msg.as_ref()));
    }

    /// Render a planned cargo invocation. Caller gates on `is_verbose`.
    pub(crate) fn command(&self, cmd: &str) {
        self.diagln(format!("  {} {cmd}", "$".dimmed()));
    }

    /// Render an always-visible status note.
    pub(crate) fn note(&self, msg: &str) {
        self.diagln(format!("  {msg}"));
    }

    #[expect(
        clippy::print_stdout,
        reason = "Reporter writes each section as one suspended progress update."
    )]
    pub(crate) fn print_section(&self, label: &str, output: &str, passed: bool) {
        let (header, divider, pipe) = if passed {
            (
                format!(" ✔ {} OUTPUT ", label.to_uppercase())
                    .green()
                    .bold()
                    .to_string(),
                "━".repeat(40).green().dimmed().to_string(),
                "│".green().dimmed().to_string(),
            )
        } else {
            (
                format!(" ✖ {} ERRORS ", label.to_uppercase())
                    .red()
                    .bold()
                    .to_string(),
                "━".repeat(40).red().dimmed().to_string(),
                "│".red().dimmed().to_string(),
            )
        };
        let output = output.trim();

        // Batch the whole section under one `suspend` so a long dump does
        // not trigger N pause/redraw cycles of the spinner block.
        self.mp.suspend(|| {
            println!();
            println!("{header}");
            println!("{divider}");
            if output.is_empty() {
                println!(" {pipe} {}", "(no output)".dimmed());
            } else {
                for line in output.lines() {
                    println!(" {pipe} {line}");
                }
            }
            println!();
        });
    }

    /// Final footer. Lists failing labels, or reports total on success.
    pub(crate) fn summary(&self, total: usize, failures: &[&str]) {
        self.reportln("");
        if failures.is_empty() {
            let msg = format!("OK: {total}/{total} checks passed").green().bold();
            self.reportln(format!("  {msg}"));
        } else {
            let failed = failures.len();
            let list = failures.join(", ");
            let msg = format!("Failed: {failed}/{total} ({list})").red().bold();
            self.reportln(format!("  {msg}"));
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn outcome_predicates_track_status() {
        let skip = CheckOutcome::skipped();
        assert_eq!(skip.status, TaskStatus::Skip);
        assert_eq!(skip.output, "");
        assert!(!skip.passed() && !skip.failed());

        let pass = CheckOutcome {
            status: TaskStatus::Pass,
            output: String::new(),
        };
        assert!(pass.passed() && !pass.failed());

        let fail = CheckOutcome {
            status: TaskStatus::Fail,
            output: String::new(),
        };
        assert!(fail.failed() && !fail.passed());
    }

    #[test]
    fn stream_routing_keeps_statuses_once_and_preserves_diagnostics() {
        const CHILD: &str = "LOCKPICK_REPORTER_STREAMS";
        if let Ok(mode) = std::env::var(CHILD) {
            let stderr_tty = mode.starts_with('1');
            let stdout_tty = mode.ends_with('1');
            let reporter = Reporter::new(true, stderr_tty, stdout_tty);
            for (label, status, tag) in [
                ("compile", TaskStatus::Pass, "PASS"),
                ("test", TaskStatus::Fail, "FAIL"),
                ("coverage", TaskStatus::Skip, "SKIP"),
            ] {
                let spinner = reporter.add_spinner(label);
                reporter.finish_spinner(&spinner, label, status);
                assert!(spinner.is_finished());
                if stderr_tty {
                    assert!(spinner.message().contains(tag));
                }
            }
            reporter.note("diagnostic-marker");
            reporter.print_section("test", "first line\nsecond line", false);
            reporter.print_section("compile", "", true);
            reporter.summary(3, &["test"]);
            return;
        }
        for mode in ["00", "01", "10", "11"] {
            let out = crate::test_process::bounded_output(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "reporter::tests::stream_routing_keeps_statuses_once_and_preserves_diagnostics", "--nocapture"])
                    .env(CHILD, mode),
            ).unwrap();
            let report = String::from_utf8_lossy(&out.stdout);
            let diagnostic = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{report}\n{diagnostic}");
            for tag in ["PASS", "FAIL", "SKIP"] {
                assert_eq!(
                    report.matches(tag).count(),
                    usize::from(mode != "11"),
                    "{report}"
                );
            }
            assert!(report.contains("first line") && report.contains("second line"));
            assert!(report.contains("(no output)"));
            assert!(report.contains("Failed: 1/3 (test)"));
            assert!(!report.contains("diagnostic-marker"));
            if mode.starts_with('0') {
                assert!(diagnostic.contains("diagnostic-marker"));
            }
        }
    }

    #[test]
    fn malformed_progress_template_still_finishes() {
        let spinner = ProgressBar::hidden();
        spinner.set_style(parse_template("{msg:invalid}"));
        spinner.finish_with_message("done");
        assert!(spinner.is_finished());
        assert_eq!(spinner.message(), "done");
    }
}
