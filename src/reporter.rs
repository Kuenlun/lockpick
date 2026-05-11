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

/// Visual flavour of a section banner. Skip statuses don't print anything,
/// so they're represented as `None` upstream — collapsing the runtime
/// "no banner for Skip" decision into a single check instead of a Skip
/// arm in every match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SectionKind {
    Pass,
    Fail,
}

impl SectionKind {
    const fn from_status(status: TaskStatus) -> Option<Self> {
        match status {
            TaskStatus::Pass => Some(Self::Pass),
            TaskStatus::Fail => Some(Self::Fail),
            TaskStatus::Skip => None,
        }
    }

    fn header(self, label: &str) -> String {
        match self {
            Self::Pass => format!(" ✔ {} OUTPUT ", label.to_uppercase())
                .green()
                .bold()
                .to_string(),
            Self::Fail => format!(" ✖ {} ERRORS ", label.to_uppercase())
                .red()
                .bold()
                .to_string(),
        }
    }

    fn divider(self) -> String {
        let raw = "━".repeat(40);
        match self {
            Self::Pass => raw.green().dimmed().to_string(),
            Self::Fail => raw.red().dimmed().to_string(),
        }
    }

    fn pipe(self) -> String {
        match self {
            Self::Pass => "│".green().dimmed().to_string(),
            Self::Fail => "│".red().dimmed().to_string(),
        }
    }
}

#[allow(clippy::literal_string_with_formatting_args)]
const SPIN_TEMPLATE: &str = "  {msg:<8} {spinner:.cyan}";
const DONE_TEMPLATE: &str = "  {msg}";
const TICK_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

/// Build an indicatif `ProgressStyle`. The default templates we ship with
/// always parse, so an invalid template falls back to indicatif's default
/// style rather than propagating an error — that keeps the `Reporter`
/// constructor infallible from the caller's perspective. Tests cover both
/// branches by passing a known-bad template.
fn parse_template(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template).unwrap_or_else(|_| ProgressStyle::default_spinner())
}

impl Reporter {
    /// Construct a Reporter using the default templates. `is_tty` selects
    /// between progress-bar rendering (true) and plain stderr (false).
    /// Tests pass an explicit boolean to drive both branches.
    #[must_use]
    pub fn new(is_verbose: bool, is_tty: bool) -> Self {
        let spin_style = parse_template(SPIN_TEMPLATE).tick_chars(TICK_CHARS);
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

    /// Auto-detecting constructor used by production code paths.
    #[must_use]
    pub fn auto(is_verbose: bool) -> Self {
        Self::new(is_verbose, std::io::stderr().is_terminal())
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
            pb.finish_with_message(format!("{label:<8} {tag}"));
        } else {
            pb.finish_and_clear();
            eprintln!("  {label:<8} {tag}");
        }
    }

    pub fn println(&self, msg: impl AsRef<str>) {
        if self.is_tty {
            self.mp.println(msg).ok();
        } else {
            eprintln!("{}", msg.as_ref());
        }
    }

    /// Print the cargo invocation about to run. Hidden unless `--verbose`.
    pub fn command(&self, cmd: &str) {
        if self.is_verbose {
            self.println(format!("  {} {cmd}", "$".dimmed()));
        }
    }

    /// Print an informational message; hidden unless `--verbose`.
    pub fn info(&self, msg: &str) {
        if self.is_verbose {
            self.println(format!("  {} {msg}", "info:".cyan().bold()));
        }
    }

    /// Print a message that is always visible (used for explanatory notes
    /// such as "All checks disabled, nothing to run").
    pub fn note(&self, msg: &str) {
        self.println(format!("  {msg}"));
    }

    pub fn print_section(&self, label: &str, output: &str, status: TaskStatus) {
        let Some(kind) = SectionKind::from_status(status) else {
            return;
        };
        let output = output.trim();

        self.println("");
        self.println(kind.header(label));

        if output.is_empty() {
            self.println(format!("  {}", "(no output)".dimmed()));
            return;
        }

        self.println(kind.divider());
        let pipe = kind.pipe();
        for line in output.lines() {
            self.println(format!(" {pipe} {line}"));
        }

        self.println("");
    }

    /// Final summary line. Lists the labels that failed when any did,
    /// otherwise reports total checks that passed.
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
mod tests {
    use super::*;

    #[test]
    fn check_outcome_passed_is_true_only_when_status_is_pass() {
        let pass = CheckOutcome {
            status: TaskStatus::Pass,
            output: String::new(),
        };
        let fail = CheckOutcome {
            status: TaskStatus::Fail,
            output: String::new(),
        };
        let skip = CheckOutcome::skipped();
        assert!(pass.passed());
        assert!(!fail.passed());
        assert!(!skip.passed());
    }

    #[test]
    fn check_outcome_failed_is_true_only_when_status_is_fail() {
        let pass = CheckOutcome {
            status: TaskStatus::Pass,
            output: String::new(),
        };
        let fail = CheckOutcome {
            status: TaskStatus::Fail,
            output: String::new(),
        };
        let skip = CheckOutcome::skipped();
        assert!(!pass.failed());
        assert!(fail.failed());
        assert!(!skip.failed());
    }

    #[test]
    fn skipped_outcome_has_empty_output_and_skip_status() {
        let s = CheckOutcome::skipped();
        assert!(s.output.is_empty());
        assert_eq!(s.status, TaskStatus::Skip);
    }

    #[test]
    fn reporter_new_records_supplied_flags() {
        let r = Reporter::new(true, false);
        assert!(r.is_verbose);
        assert!(!r.is_tty);

        let r = Reporter::new(false, true);
        assert!(!r.is_verbose);
        assert!(r.is_tty);
    }

    #[test]
    fn reporter_auto_constructor_runs() {
        let r = Reporter::auto(false);
        assert!(!r.is_verbose);
    }

    #[test]
    fn parse_template_falls_back_to_default_for_an_invalid_template() {
        // `{}` is rejected by indicatif's parser; we fall back gracefully
        // so the production constructor can stay infallible. The valid-
        // template branch is exercised implicitly by every `Reporter::new`
        // call in this module.
        let _ = parse_template("{}");
    }

    /// Exercises both branches of `add_spinner` and `finish_spinner`. We
    /// can't observe what indicatif writes, so the assertion is the
    /// status-transition contract: after `finish_spinner` the bar reads
    /// as finished, which is the signal the rest of the pipeline waits
    /// on. `println` is also driven on both branches as a side effect.
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

    // The reporter writes to stderr / `MultiProgress` without exposing
    // a swappable sink, so the tests below cannot assert on the rendered
    // output. They instead drive every branch on both the verbose and
    // tty axes to lock in the no-panic / no-deadlock contract that the
    // rest of the pipeline relies on. Pure formatting is verified via
    // `SectionKind` below.

    #[test]
    fn verbose_gated_methods_drive_both_modes() {
        for is_verbose in [true, false] {
            let r = Reporter::new(is_verbose, false);
            r.command("cargo check");
            r.info("just so you know");
            r.note("everything skipped");
        }
    }

    #[test]
    fn print_section_covers_every_status_in_both_tty_modes() {
        for is_tty in [true, false] {
            let r = Reporter::new(false, is_tty);
            r.print_section("clippy", "fine\nmore\n", TaskStatus::Pass);
            r.print_section("fmt", "bad\n", TaskStatus::Fail);
            r.print_section("test", "anything", TaskStatus::Skip);
            // Empty output triggers the "(no output)" marker branch.
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

    #[test]
    fn section_kind_maps_pass_and_fail_to_some_skip_to_none() {
        assert_eq!(
            SectionKind::from_status(TaskStatus::Pass),
            Some(SectionKind::Pass)
        );
        assert_eq!(
            SectionKind::from_status(TaskStatus::Fail),
            Some(SectionKind::Fail)
        );
        assert_eq!(SectionKind::from_status(TaskStatus::Skip), None);
    }

    #[test]
    fn section_kind_renders_distinct_decorations_for_pass_and_fail() {
        for kind in [SectionKind::Pass, SectionKind::Fail] {
            let h = kind.header("clippy");
            assert!(h.contains("CLIPPY"));
            assert!(!kind.divider().is_empty());
            assert!(!kind.pipe().is_empty());
        }
    }
}
