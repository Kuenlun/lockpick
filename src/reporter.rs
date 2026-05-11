// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::io::IsTerminal;
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::error::LockpickError;

#[derive(Clone, Copy)]
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
    pub mp: MultiProgress,
    spin_style: ProgressStyle,
    done_style: ProgressStyle,
    pub is_tty: bool,
}

impl Reporter {
    pub fn new() -> Result<Self, LockpickError> {
        #[allow(clippy::literal_string_with_formatting_args)]
        let spin_template = "  {msg:<8} {spinner:.cyan}";
        let spin_style = ProgressStyle::with_template(spin_template)?.tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");
        let done_style = ProgressStyle::with_template("  {msg}")?;

        let is_tty = std::io::stderr().is_terminal();
        let mp = if is_tty {
            MultiProgress::new()
        } else {
            MultiProgress::with_draw_target(ProgressDrawTarget::hidden())
        };

        Ok(Self {
            mp,
            spin_style,
            done_style,
            is_tty,
        })
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

    pub fn print_section(&self, label: &str, output: &str, status: TaskStatus) {
        let output = output.trim();

        self.println("");

        let header = match status {
            TaskStatus::Pass => format!(" ✔ {} OUTPUT ", label.to_uppercase())
                .green()
                .bold()
                .to_string(),
            TaskStatus::Fail => format!(" ✖ {} ERRORS ", label.to_uppercase())
                .red()
                .bold()
                .to_string(),
            TaskStatus::Skip => return,
        };
        self.println(header);

        if output.is_empty() {
            self.println(format!("  {}", "(no output)".dimmed()));
            return;
        }

        let divider_raw = "━".repeat(40);
        let divider = match status {
            TaskStatus::Pass => divider_raw.green().dimmed().to_string(),
            TaskStatus::Fail => divider_raw.red().dimmed().to_string(),
            TaskStatus::Skip => return,
        };
        self.println(divider);

        let pipe = match status {
            TaskStatus::Pass => "│".green().dimmed().to_string(),
            TaskStatus::Fail => "│".red().dimmed().to_string(),
            TaskStatus::Skip => return,
        };
        for line in output.lines() {
            self.println(format!(" {pipe} {line}"));
        }

        self.println("");
    }
}
