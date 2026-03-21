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

pub fn run(_cli: Cli) -> Result<(), LockpickError> {
    let checks: &[(&str, &[&str])] = &[
        ("check", &["--workspace", "--all-targets", "--all-features"]),
        (
            "clippy",
            &["--workspace", "--all-targets", "--all-features"],
        ),
        ("fmt", &["--check"]),
        ("test", &["--workspace", "--all-targets", "--all-features"]),
    ];

    let mp = MultiProgress::new();

    let spinner_style = ProgressStyle::with_template("  {spinner:.cyan}  {msg}")
        .unwrap_or_else(|_| unreachable!())
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]);

    let done_style = ProgressStyle::with_template("  {msg}").unwrap_or_else(|_| unreachable!());

    let spinners: Vec<_> = checks
        .iter()
        .map(|(subcommand, _)| {
            let pb = mp.add(ProgressBar::new_spinner());
            pb.set_style(spinner_style.clone());
            pb.set_message(format!("{subcommand:<8}"));
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        })
        .collect();

    let failure_count = thread::scope(|s| {
        // Collect is required: without it the iterator is lazy and threads
        // would be spawned and joined one at a time, defeating parallelism.
        #[allow(clippy::needless_collect)]
        let handles: Vec<_> = checks
            .iter()
            .enumerate()
            .map(|(i, (subcommand, args))| {
                let pb = &spinners[i];
                let style = done_style.clone();
                s.spawn(move || {
                    let success = run_cargo(subcommand, args).is_ok_and(|status| status.success());

                    let label = format!("{subcommand:<8}");
                    pb.set_style(style);

                    if success {
                        pb.finish_with_message(format!(
                            "{}  {label}{}",
                            "✓".green().bold(),
                            "PASS".green().bold(),
                        ));
                    } else {
                        pb.finish_with_message(format!(
                            "{}  {label}{}",
                            "✗".red().bold(),
                            "FAIL".red().bold(),
                        ));
                    }

                    success
                })
            })
            .collect();

        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|&passed| !passed)
            .count()
    });

    if failure_count > 0 {
        return Err(LockpickError::ChecksFailed(failure_count));
    }

    Ok(())
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
