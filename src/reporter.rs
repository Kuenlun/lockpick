// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
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
    /// Stderr is a TTY. Drives spinner rendering and routes `diag`
    /// writes through `MultiProgress` so they interleave cleanly with
    /// the active spinner block.
    is_tty: bool,
    /// Stdout is a TTY. When false, the report stream is being captured
    /// (file, pipe, CI), so the spinner keeps a visible final state on
    /// stderr instead of clearing. Otherwise the interactive user would
    /// only see spinners disappear.
    stdout_is_tty: bool,
    pub is_verbose: bool,
}

/// Column width used to align check labels in spinners and status lines.
/// Pinned against the longest known label by a test in `runner`.
pub const LABEL_WIDTH: usize = 10;

const DONE_TEMPLATE: &str = "  {msg}";
const TICK_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

fn spin_template() -> String {
    format!("  {{msg:<{LABEL_WIDTH}}} {{spinner:.cyan}}")
}

/// Parse an indicatif template, falling back to the default spinner so
/// the caller stays infallible under `clippy::expect_used`.
fn parse_template(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template).unwrap_or_else(|_| ProgressStyle::default_spinner())
}

impl Reporter {
    /// Build a [`Reporter`] with the TTY state of stdout and stderr
    /// probed from the process's own streams.
    #[cfg_attr(test, allow(dead_code))]
    #[must_use]
    pub fn auto(is_verbose: bool) -> Self {
        Self::new(
            is_verbose,
            std::io::stderr().is_terminal(),
            std::io::stdout().is_terminal(),
        )
    }

    /// Build a [`Reporter`]. `is_tty` enables progress-bar rendering on
    /// stderr. `stdout_is_tty` controls whether the per-check spinner
    /// keeps a visible final state (stdout captured) or clears so the
    /// report on stdout is the sole record (stdout on-terminal).
    #[must_use]
    pub fn new(is_verbose: bool, is_tty: bool, stdout_is_tty: bool) -> Self {
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

    /// Finish a spinner and emit the matching status line.
    ///
    /// Three TTY cases, each with a different routing:
    ///
    /// * Stderr TTY, stdout TTY (pure interactive). Anchor the spinner
    ///   in place with `finish_with_message`, so siblings still running
    ///   in the `MultiProgress` band do not shift up to fill its row.
    ///   Skip the `reportln`: the report stream is the same terminal as
    ///   the spinner, and a `mp.suspend` write would clear-and-redraw
    ///   the active band plus duplicate the line above it.
    /// * Stderr TTY, stdout captured (`lockpick > report.txt`). Anchor
    ///   the spinner so the user keeps a visible final state on stderr,
    ///   and emit the line on stdout for the capture.
    /// * Stderr not a TTY. The spinner draw target is hidden, so the
    ///   clear is a no-op. Emit the line on stdout as the sole record.
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

    /// Write a line to the report stream (stdout): status lines, section
    /// dumps, the final summary. `MultiProgress::suspend` pauses spinner
    /// drawing for the write so the two streams do not stomp on each
    /// other when both render to the same terminal.
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

    pub fn print_section(&self, label: &str, output: &str, status: TaskStatus) {
        let (header, divider, pipe) = match status {
            TaskStatus::Pass => (
                format!(" ✔ {} OUTPUT ", label.to_uppercase())
                    .green()
                    .bold()
                    .to_string(),
                "━".repeat(40).green().dimmed().to_string(),
                "│".green().dimmed().to_string(),
            ),
            TaskStatus::Fail => (
                format!(" ✖ {} ERRORS ", label.to_uppercase())
                    .red()
                    .bold()
                    .to_string(),
                "━".repeat(40).red().dimmed().to_string(),
                "│".red().dimmed().to_string(),
            ),
            TaskStatus::Skip => return,
        };
        let output = output.trim();

        self.reportln("");
        self.reportln(header);
        self.reportln(divider);

        if output.is_empty() {
            self.reportln(format!(" {pipe} {}", "(no output)".dimmed()));
        } else {
            for line in output.lines() {
                self.reportln(format!(" {pipe} {line}"));
            }
        }

        self.reportln("");
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
    fn auto_delegates_to_new_and_propagates_verbose_flag() {
        for verbose in [false, true] {
            let r = Reporter::auto(verbose);
            assert_eq!(r.is_verbose, verbose);
            let pb = r.add_spinner("probe");
            r.finish_spinner(&pb, "probe", TaskStatus::Pass);
            assert!(pb.is_finished());
        }
    }

    #[test]
    fn parse_template_falls_back_to_default_for_an_invalid_template() {
        // `{}` is rejected by indicatif's parser.
        let _ = parse_template("{}");
    }

    #[test]
    fn finish_spinner_drives_every_status_across_the_tty_matrix() {
        // Every combination of (stderr TTY, stdout TTY) hits a different
        // arm of the spinner-finish branching, so cover all four.
        for is_tty in [true, false] {
            for stdout_is_tty in [true, false] {
                let r = Reporter::new(false, is_tty, stdout_is_tty);
                for status in [TaskStatus::Pass, TaskStatus::Fail, TaskStatus::Skip] {
                    let pb = r.add_spinner("clippy");
                    assert!(!pb.is_finished());
                    r.finish_spinner(&pb, "clippy", status);
                    assert!(pb.is_finished());
                }
            }
        }
    }

    #[test]
    fn print_section_covers_every_status_in_both_tty_modes() {
        for is_tty in [true, false] {
            let r = Reporter::new(false, is_tty, false);
            r.print_section("clippy", "fine\nmore\n", TaskStatus::Pass);
            r.print_section("fmt", "bad\n", TaskStatus::Fail);
            r.print_section("test", "anything", TaskStatus::Skip);
            r.print_section("doc", "", TaskStatus::Pass);
            r.print_section("audit", "", TaskStatus::Fail);
        }
    }

    #[test]
    fn summary_handles_ok_and_failure_footers() {
        let r = Reporter::new(false, false, false);
        r.summary(5, &[]);
        r.summary(5, &["fmt", "clippy"]);
    }

    /// Drives `diagln` (and its public sugar `command`/`note`) through
    /// both branches: `MultiProgress::println` in TTY mode and `eprintln!`
    /// in non-TTY mode. Without this, the TTY arm has no coverage because
    /// every other test that hits a banner or note runs in non-TTY mode.
    #[test]
    fn diagln_routes_through_multiprogress_in_tty_mode_and_eprintln_otherwise() {
        for is_tty in [true, false] {
            let r = Reporter::new(false, is_tty, false);
            r.command("cargo check");
            r.note("--skip foo has no effect");
        }
    }
}
