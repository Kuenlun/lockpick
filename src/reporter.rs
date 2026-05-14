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
    is_tty: bool,
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
    /// Build a [`Reporter`] with `is_tty` probed from stderr.
    #[cfg_attr(test, allow(dead_code))]
    #[must_use]
    pub fn auto(is_verbose: bool) -> Self {
        Self::new(is_verbose, std::io::stderr().is_terminal())
    }

    /// Build a [`Reporter`]. `is_tty = true` enables progress-bar
    /// rendering; `false` falls back to plain stderr lines.
    #[must_use]
    pub fn new(is_verbose: bool, is_tty: bool) -> Self {
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
            eprintln!("  {label:<LABEL_WIDTH$} {tag}");
        }
    }

    pub fn println(&self, msg: impl AsRef<str>) {
        if self.is_tty {
            self.mp.println(msg).ok();
        } else {
            eprintln!("{}", msg.as_ref());
        }
    }

    /// Render a planned cargo invocation. Caller gates on `is_verbose`.
    pub fn command(&self, cmd: &str) {
        self.println(format!("  {} {cmd}", "$".dimmed()));
    }

    /// Render an always-visible status note.
    pub fn note(&self, msg: &str) {
        self.println(format!("  {msg}"));
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

        self.println("");
        self.println(header);

        if output.is_empty() {
            self.println(format!("  {}", "(no output)".dimmed()));
            return;
        }

        self.println(divider);
        for line in output.lines() {
            self.println(format!(" {pipe} {line}"));
        }

        self.println("");
    }

    /// Final footer. Lists failing labels, or reports total on success.
    pub fn summary(&self, total: usize, failures: &[&str]) {
        self.println("");
        if failures.is_empty() {
            let msg = format!("OK: {total}/{total} checks passed").green().bold();
            self.println(format!("  {msg}"));
        } else {
            let failed = failures.len();
            let list = failures.join(", ");
            let msg = format!("Failed: {failed}/{total} ({list})").red().bold();
            self.println(format!("  {msg}"));
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
    fn finish_spinner_drives_both_modes_through_every_status() {
        for is_tty in [true, false] {
            let r = Reporter::new(false, is_tty);
            for status in [TaskStatus::Pass, TaskStatus::Fail, TaskStatus::Skip] {
                let pb = r.add_spinner("clippy");
                assert!(!pb.is_finished());
                r.finish_spinner(&pb, "clippy", status);
                assert!(pb.is_finished());
            }
        }
    }

    #[test]
    fn print_section_covers_every_status_in_both_tty_modes() {
        for is_tty in [true, false] {
            let r = Reporter::new(false, is_tty);
            r.print_section("clippy", "fine\nmore\n", TaskStatus::Pass);
            r.print_section("fmt", "bad\n", TaskStatus::Fail);
            r.print_section("test", "anything", TaskStatus::Skip);
            r.print_section("doc", "", TaskStatus::Pass);
            r.print_section("audit", "", TaskStatus::Fail);
        }
    }

    #[test]
    fn summary_handles_ok_and_failure_footers() {
        let r = Reporter::new(false, false);
        r.summary(5, &[]);
        r.summary(5, &["fmt", "clippy"]);
    }
}
