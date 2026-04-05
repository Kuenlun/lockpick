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

use std::io::{IsTerminal, Write};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle, TermLike};

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

/// Minimal [`TermLike`] that writes to stderr without any TTY checks.
///
/// Cursor movements and line clearing are no-ops so that piped output
/// stays clean (no ANSI escape noise).
#[derive(Debug)]
struct PlainStderr;

impl TermLike for PlainStderr {
    fn width(&self) -> u16 {
        80
    }
    fn move_cursor_up(&self, _n: usize) -> std::io::Result<()> {
        Ok(())
    }
    fn move_cursor_down(&self, _n: usize) -> std::io::Result<()> {
        Ok(())
    }
    fn move_cursor_right(&self, _n: usize) -> std::io::Result<()> {
        Ok(())
    }
    fn move_cursor_left(&self, _n: usize) -> std::io::Result<()> {
        Ok(())
    }
    fn write_line(&self, s: &str) -> std::io::Result<()> {
        writeln!(std::io::stderr(), "{s}")
    }
    fn write_str(&self, s: &str) -> std::io::Result<()> {
        write!(std::io::stderr(), "{s}")
    }
    fn clear_line(&self) -> std::io::Result<()> {
        Ok(())
    }
    fn flush(&self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

// Indicatif
struct Reporter {
    mp: MultiProgress,
    spin_style: ProgressStyle,
    done_style: ProgressStyle,
    is_tty: bool,
}

impl Reporter {
    fn new() -> Result<Self, LockpickError> {
        #[allow(clippy::literal_string_with_formatting_args)]
        let spin_template = "  {msg:<8} {spinner:.cyan}";
        let spin_style = ProgressStyle::with_template(spin_template)?.tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");
        let done_style = ProgressStyle::with_template("  {msg}")?;

        let is_tty = std::io::stderr().is_terminal();
        let mp = if is_tty {
            MultiProgress::new()
        } else {
            MultiProgress::with_draw_target(ProgressDrawTarget::term_like_with_hz(
                Box::new(PlainStderr),
                20,
            ))
        };

        Ok(Self {
            mp,
            spin_style,
            done_style,
            is_tty,
        })
    }

    fn add_spinner(&self, label: &str) -> ProgressBar {
        let pb = self.mp.add(ProgressBar::new_spinner());
        pb.set_style(self.spin_style.clone());
        pb.set_message(label.to_string());
        if self.is_tty {
            pb.enable_steady_tick(Duration::from_millis(80));
        }
        pb
    }

    fn finish_spinner(&self, pb: &ProgressBar, label: &str, status: TaskStatus) {
        let tag = match status {
            TaskStatus::Pass => "PASS".green().bold(),
            TaskStatus::Fail => "FAIL".red().bold(),
            TaskStatus::Skip => "SKIP".yellow().bold(),
        };
        pb.set_style(self.done_style.clone());
        pb.finish_with_message(format!("{label:<8} {tag}"));
    }

    fn print_section(&self, label: &str, output: &str, status: TaskStatus) {
        let output = output.trim();

        self.mp.println("").ok();

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
        self.mp.println(header).ok();

        if output.is_empty() {
            self.mp
                .println(format!("  {}", "(no output)".dimmed()))
                .ok();
            return;
        }

        let divider_raw = "━".repeat(40);
        let divider = match status {
            TaskStatus::Pass => divider_raw.green().dimmed().to_string(),
            TaskStatus::Fail => divider_raw.red().dimmed().to_string(),
            TaskStatus::Skip => return,
        };
        self.mp.println(divider).ok();

        let pipe = match status {
            TaskStatus::Pass => "│".green().dimmed().to_string(),
            TaskStatus::Fail => "│".red().dimmed().to_string(),
            TaskStatus::Skip => return,
        };
        for line in output.lines() {
            self.mp.println(format!(" {pipe} {line}")).ok();
        }

        self.mp.println("").ok();
    }
}

pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let reporter = Reporter::new()?;
    crate::logger::init(cli.verbose, &reporter.mp);

    let run_check = !cli.skips(&SkipOption::Check);
    let tasks = build_tasks(cli);

    if !run_check && tasks.is_empty() {
        log::info!("All checks disabled, nothing to run");
        return Ok(());
    }

    // Create all spinners upfront so every stage is visible from the start.
    let check_pb = if run_check {
        Some(reporter.add_spinner("check"))
    } else {
        None
    };
    let task_pbs: Vec<ProgressBar> = tasks
        .iter()
        .map(|t| reporter.add_spinner(t.label))
        .collect();
    let cov_pb = if cli.opt_in.coverage {
        Some(reporter.add_spinner("coverage"))
    } else {
        None
    };

    // Phase 1a: run cargo check first (gate for remaining tasks)
    let check_result = if run_check {
        let result = run_task("check", COMMON_ARGS);
        if let Some(pb) = &check_pb {
            reporter.finish_spinner(pb, "check", result.0);
        }
        Some(result)
    } else {
        None
    };

    let check_passed = check_result
        .as_ref()
        .is_none_or(|(s, _)| matches!(s, TaskStatus::Pass));

    // Phase 1b: run remaining tasks in parallel (only if check passed)
    let task_results: Vec<(bool, String)> = if check_passed {
        run_parallel(&tasks)
    } else {
        tasks.iter().map(|_| (false, String::new())).collect()
    };
    for (i, pb) in task_pbs.iter().enumerate() {
        let status = if !check_passed {
            TaskStatus::Skip
        } else if task_results[i].0 {
            TaskStatus::Pass
        } else {
            TaskStatus::Fail
        };
        reporter.finish_spinner(pb, tasks[i].label, status);
    }

    let cov_result: Option<(TaskStatus, String)> = if cli.opt_in.coverage {
        let tests_passed = check_passed
            && tasks
                .iter()
                .zip(&task_results)
                .find(|(t, _)| t.label == "test")
                .is_some_and(|(_, (passed, _))| *passed);

        let result = if tests_passed {
            let threshold = cli.opt_in.min_coverage.to_string();
            run_task("llvm-cov", &["report", "--fail-under-lines", &threshold])
        } else {
            (TaskStatus::Skip, String::new())
        };
        if let Some(pb) = &cov_pb {
            reporter.finish_spinner(pb, "coverage", result.0);
        }
        Some(result)
    } else {
        None
    };

    let failure_count = report_results(
        &reporter,
        cli.verbose,
        check_result.as_ref(),
        &tasks,
        &task_results,
        check_passed,
        cov_result.as_ref(),
    );

    if failure_count > 0 {
        return Err(LockpickError::ChecksFailed(failure_count));
    }

    Ok(())
}

fn report_results(
    reporter: &Reporter,
    verbose: u8,
    check_result: Option<&(TaskStatus, String)>,
    tasks: &[Task],
    task_results: &[(bool, String)],
    check_passed: bool,
    cov_result: Option<&(TaskStatus, String)>,
) -> usize {
    // Print output sections (PASS first, then FAIL).
    if verbose >= 1 {
        if let Some((TaskStatus::Pass, output)) = check_result {
            reporter.print_section("check", output, TaskStatus::Pass);
        }
        for (task, (passed, output)) in tasks.iter().zip(task_results) {
            if *passed && check_passed {
                reporter.print_section(task.label, output, TaskStatus::Pass);
            }
        }
        if let Some((TaskStatus::Pass, output)) = cov_result {
            reporter.print_section("coverage", output, TaskStatus::Pass);
        }
    }

    if let Some((TaskStatus::Fail, output)) = check_result {
        reporter.print_section("check", output, TaskStatus::Fail);
    }
    for (task, (passed, output)) in tasks.iter().zip(task_results) {
        if !passed && check_passed {
            reporter.print_section(task.label, output, TaskStatus::Fail);
        }
    }
    if let Some((TaskStatus::Fail, output)) = cov_result {
        reporter.print_section("coverage", output, TaskStatus::Fail);
    }

    // Count failures.
    let mut count = 0;
    if matches!(check_result, Some((TaskStatus::Fail, _))) {
        count += 1;
    }
    if check_passed {
        count += task_results.iter().filter(|(passed, _)| !passed).count();
    }
    if matches!(cov_result, Some((TaskStatus::Fail, _))) {
        count += 1;
    }
    count
}

fn build_tasks(cli: &Cli) -> Vec<Task> {
    let mut tasks = Vec::new();

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

fn run_parallel(tasks: &[Task]) -> Vec<(bool, String)> {
    thread::scope(|s| {
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
    })
}

fn run_task(subcommand: &str, args: &[&str]) -> (TaskStatus, String) {
    let (passed, output) = match run_cargo(subcommand, args) {
        Ok((status, out)) => (status.success(), out),
        Err(_) => (false, String::new()),
    };
    let status = if passed {
        TaskStatus::Pass
    } else {
        TaskStatus::Fail
    };
    (status, output)
}

fn run_cargo(subcommand: &str, args: &[&str]) -> Result<(ExitStatus, String), LockpickError> {
    log::info!("cargo {subcommand} {}", args.join(" "));

    let output = Command::new("cargo")
        .arg(subcommand)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut combined = stdout.into_owned();
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&stderr);

    Ok((output.status, combined))
}
