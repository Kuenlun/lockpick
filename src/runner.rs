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

use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::cli::Cli;
use crate::error::LockpickError;

struct Check {
    name: &'static str,
    args: &'static [&'static str],
}

pub fn run(cli: &Cli) -> Result<(), LockpickError> {
    let checks = enabled_checks(cli);

    if checks.is_empty() {
        log::info!("All checks disabled, nothing to run");
        return Ok(());
    }

    let failure_count = run_parallel(&checks)?;

    if failure_count > 0 {
        return Err(LockpickError::ChecksFailed(failure_count));
    }

    Ok(())
}

fn enabled_checks(cli: &Cli) -> Vec<Check> {
    const COMMON_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];

    let mut checks = Vec::new();

    if cli.opt_in.check {
        checks.push(Check {
            name: "check",
            args: COMMON_ARGS,
        });
    }
    if !cli.opt_out.no_clippy {
        checks.push(Check {
            name: "clippy",
            args: COMMON_ARGS,
        });
    }
    if !cli.opt_out.no_fmt {
        checks.push(Check {
            name: "fmt",
            args: &["--check"],
        });
    }
    if !cli.opt_out.no_test {
        checks.push(Check {
            name: "test",
            args: COMMON_ARGS,
        });
    }

    checks
}

fn run_parallel(checks: &[Check]) -> Result<usize, LockpickError> {
    let mp = MultiProgress::new();

    let spinner_tpl = String::from_utf8_lossy(b"  {msg:<8} {spinner:.cyan}");
    let spinner_style = ProgressStyle::with_template(&spinner_tpl)?.tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");

    let done_tpl = String::from_utf8_lossy(b"  {msg}");
    let done_style = ProgressStyle::with_template(&done_tpl)?;

    let spinners: Vec<ProgressBar> = checks
        .iter()
        .map(|check| {
            let pb = mp.add(ProgressBar::new_spinner());
            pb.set_style(spinner_style.clone());
            pb.set_message(check.name.to_string());
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        })
        .collect();

    let failure_count = thread::scope(|s| {
        let mut handles = Vec::with_capacity(checks.len());

        for (check, pb) in checks.iter().zip(&spinners) {
            let style = done_style.clone();
            handles.push(s.spawn(move || {
                let passed = run_cargo(check.name, check.args).is_ok_and(|status| status.success());

                pb.set_style(style);
                if passed {
                    pb.finish_with_message(format!("{:<8} {}", check.name, "PASS".green().bold()));
                } else {
                    pb.finish_with_message(format!("{:<8} {}", check.name, "FAIL".red().bold()));
                }

                passed
            }));
        }

        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|&passed| !passed)
            .count()
    });

    Ok(failure_count)
}

fn run_cargo(subcommand: &str, args: &[&str]) -> Result<ExitStatus, LockpickError> {
    log::debug!("Running cargo {subcommand}");

    Command::new("cargo")
        .arg(subcommand)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(LockpickError::from)
}
