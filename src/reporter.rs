// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::io::IsTerminal;
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Pass,
    Fail,
    Skip,
}

pub struct CheckOutcome {
    pub status: TaskStatus,
    pub output: String,
}

impl CheckOutcome {
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self.status, TaskStatus::Pass)
    }

    #[must_use]
    pub const fn failed(&self) -> bool {
        matches!(self.status, TaskStatus::Fail)
    }

    #[must_use]
    pub const fn skipped() -> Self {
        Self {
            status: TaskStatus::Skip,
            output: String::new(),
        }
    }
}

pub struct Reporter {
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
    pub is_verbose: bool,
}

/// Column width used to align check labels in spinners and status lines.
/// Must accommodate the longest concrete `Check::label()`.
pub const LABEL_WIDTH: usize = 10;

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
    pub fn auto(is_verbose: bool) -> Self {
        let is_tty = std::io::stderr().is_terminal();
        let stdout_is_tty = std::io::stdout().is_terminal();
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

    pub fn add_spinner(&self, label: &str) -> ProgressBar {
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
    pub fn finish_spinner(&self, pb: &ProgressBar, label: &str, status: TaskStatus) {
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
    pub fn diagln(&self, msg: impl AsRef<str>) {
        if self.is_tty {
            self.mp.println(msg).ok();
        } else {
            eprintln!("{}", msg.as_ref());
        }
    }

    /// Write a line to the report stream (stdout): status lines and the
    /// final summary. `MultiProgress::suspend` pauses spinner drawing
    /// for the write so the two streams do not stomp on each other.
    /// Multi-line blocks should batch under one `suspend` (see
    /// [`Self::print_section`]) to avoid one redraw cycle per line.
    pub fn reportln(&self, msg: impl AsRef<str>) {
        self.mp.suspend(|| println!("{}", msg.as_ref()));
    }

    /// Render a planned cargo invocation. Caller gates on `is_verbose`.
    pub fn command(&self, cmd: &str) {
        self.diagln(format!("  {} {cmd}", "$".dimmed()));
    }

    /// Render an always-visible status note.
    pub fn note(&self, msg: &str) {
        self.diagln(format!("  {msg}"));
    }

    pub fn print_section(&self, label: &str, output: &str, passed: bool) {
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
    pub fn summary(&self, total: usize, failures: &[&str]) {
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
        assert!(skip.output.is_empty());
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
}
