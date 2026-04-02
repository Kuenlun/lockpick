/*!
lockpick - Rust CLI to enforce merge checks and code quality
Copyright (C) 2026  Juan Luis Leal Contreras (Kuenlun)

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::cli::{Cli, SkipOption};
use crate::error::LockpickError;

const COMMON_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];
const COV_TEST_ARGS: &[&str] = &[
    "--workspace",
    "--all-targets",
    "--all-features",
    "--no-fail-fast",
];

#[derive(Clone, Copy)]
enum TaskStatus {
    Pass,
    Fail,
    Skip,
}

struct Task {
    label: &'static str,
    subcommand: &'static str,
    args: &'static [&'static str],
}

// Indicatif
struct Reporter {
    mp: MultiProgress,
    spin_style: ProgressStyle,
}

impl Reporter {
    fn new() -> Result<Self, LockpickError> {
        #[allow(clippy::literal_string_with_formatting_args)]
        let spin_template = "  {msg:<8} {spinner:.cyan}";
        let spin_style = ProgressStyle::with_template(spin_template)?.tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

        Ok(Self {
            mp: MultiProgress::new(),
            spin_style,
        })
    }

    fn add_spinner(&self, label: &str) -> ProgressBar {
        let pb = self.mp.add(ProgressBar::new_spinner());
        pb.set_style(self.spin_style.clone());
        pb.set_message(label.to_string());
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    }

    fn finish_task(&self, pb: &ProgressBar, label: &str, status: TaskStatus) {
        let tag = match status {
            TaskStatus::Pass => "PASS".green().bold(),
            TaskStatus::Fail => "FAIL".red().bold(),
            TaskStatus::Skip => "SKIP".yellow().bold(),
        };
        let _ = self.mp.println(format!("  {label:<8} {tag}"));
        pb.finish_and_clear();
    }

    fn print_error_section(&self, label: &str, output: &str) {
        let output = output.trim();
        if output.is_empty() {
            return;
        }

        self.mp.println("").ok();

        let header = format!(" ✖ {} ERRORS ", label.to_uppercase());
        self.mp.println(header.red().bold().to_string()).ok();

        let divider = "━".repeat(40).red().dimmed().to_string();
        self.mp.println(divider).ok();

        for line in output.lines() {
            self.mp.println(format!(" │ {line}")).ok();
        }

        self.mp.println("").ok();
    }
}

pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::new()?;
    crate::logger::init(cli.verbose, &reporter.mp);

    let tasks = build_tasks(cli);

    if tasks.is_empty() {
        log::info!("All checks disabled, nothing to run");
        return Ok(());
    }

    let mut failure_count = 0;

    let results = run_parallel(&tasks, &reporter);
    failure_count += results.iter().filter(|&&passed| !passed).count();

    if cli.opt_in.coverage {
        let tests_passed = tasks
            .iter()
            .zip(&results)
            .find(|(t, _)| t.label == "test")
            .is_some_and(|(_, &passed)| passed);

        if !run_coverage_report(tests_passed, cli.opt_in.min_coverage, &reporter) {
            failure_count += 1;
        }
    }

    if failure_count > 0 {
        return Err(LockpickError::ChecksFailed(failure_count));
    }

    Ok(())
}

fn build_tasks(cli: &Cli) -> Vec<Task> {
    let mut tasks = Vec::new();

    let run_check =
        (cli.opt_in.check || cli.skips(&SkipOption::Clippy)) && !cli.skips(&SkipOption::Check);
    if run_check {
        tasks.push(Task {
            label: "check",
            subcommand: "check",
            args: COMMON_ARGS,
        });
    }
    if !cli.skips(&SkipOption::Clippy) {
        tasks.push(Task {
            label: "clippy",
            subcommand: "clippy",
            args: COMMON_ARGS,
        });
    }
    if !cli.skips(&SkipOption::Fmt) {
        tasks.push(Task {
            label: "fmt",
            subcommand: "fmt",
            args: &["--check"],
        });
    }
    if !cli.skips(&SkipOption::Test) {
        let (subcommand, args) = if cli.opt_in.coverage {
            ("llvm-cov", COV_TEST_ARGS)
        } else {
            ("test", COMMON_ARGS)
        };
        tasks.push(Task {
            label: "test",
            subcommand,
            args,
        });
    }
    if !cli.skips(&SkipOption::DocTest) && workspace_has_lib_target() {
        tasks.push(Task {
            label: "doc test",
            subcommand: "test",
            args: &["--doc", "--workspace", "--all-features"],
        });
    }

    tasks
}

fn workspace_has_lib_target() -> bool {
    Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .stderr(Stdio::null())
        .output()
        .as_ref()
        .ok()
        .and_then(|o: &Output| std::str::from_utf8(&o.stdout).ok())
        .is_some_and(|s| s.contains(r#""kind":["lib"]"#))
}

fn run_parallel(tasks: &[Task], reporter: &Reporter) -> Vec<bool> {
    let spinners: Vec<ProgressBar> = tasks
        .iter()
        .map(|t| reporter.add_spinner(t.label))
        .collect();

    let task_results: Vec<(bool, String)> = thread::scope(|s| {
        tasks
            .iter()
            .map(|task| {
                s.spawn(move || match run_cargo(task.subcommand, task.args) {
                    Ok((status, out)) => (status.success(), out),
                    Err(_) => (false, String::new()),
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });

    for (task, (passed, output)) in tasks.iter().zip(&task_results) {
        if !passed {
            reporter.print_error_section(task.label, output);
        }
    }

    for ((task, pb), (passed, _)) in tasks.iter().zip(&spinners).zip(&task_results) {
        let status = if *passed {
            TaskStatus::Pass
        } else {
            TaskStatus::Fail
        };
        reporter.finish_task(pb, task.label, status);
    }

    task_results.into_iter().map(|(passed, _)| passed).collect()
}

fn run_coverage_report(tests_passed: bool, min_coverage: u8, reporter: &Reporter) -> bool {
    let label = "coverage";
    let pb = reporter.add_spinner(label);

    if !tests_passed {
        reporter.finish_task(&pb, label, TaskStatus::Skip);
        return true;
    }

    let threshold = min_coverage.to_string();
    let (passed, output) =
        match run_cargo("llvm-cov", &["report", "--fail-under-lines", &threshold]) {
            Ok((status, out)) => (status.success(), out),
            Err(_) => (false, String::new()),
        };

    if !passed {
        reporter.print_error_section(label, &output);
    }

    let status = if passed {
        TaskStatus::Pass
    } else {
        TaskStatus::Fail
    };
    reporter.finish_task(&pb, label, status);

    passed
}

fn run_cargo(subcommand: &str, args: &[&str]) -> Result<(ExitStatus, String), LockpickError> {
    log::debug!("cargo {subcommand} {}", args.join(" "));

    let output = Command::new("cargo")
        .arg(subcommand)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if log::max_level() >= log::LevelFilter::Trace {
        for line in stdout.lines() {
            log::trace!("[{subcommand}] {line}");
        }
        for line in stderr.lines() {
            log::trace!("[{subcommand}] {line}");
        }
    }

    let mut combined = stdout.into_owned();
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&stderr);

    Ok((output.status, combined))
}
