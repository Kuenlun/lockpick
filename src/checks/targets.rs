// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Additional target/profile Clippy checks. No linking or test execution.

use super::{Check, Runner, cargo_outcome, clippy::CLIPPY_LINT_ARGS, util::BuildOptions};
use crate::config::{Config, TargetCheck};
use crate::reporter::{CheckOutcome, TaskStatus};

pub struct TargetChecks {
    commands: Vec<Vec<String>>,
}

impl TargetChecks {
    pub fn new(config: &Config, default_clippy: bool) -> Self {
        let options = BuildOptions {
            locked: config.locked,
            host_target: config.host_target,
        };
        let default = arguments(&TargetCheck::default(), options);
        let mut commands = Vec::new();
        for check in &config.target_checks {
            let args = arguments(check, options);
            if !(default_clippy && args == default || commands.contains(&args)) {
                commands.push(args);
            }
        }
        Self { commands }
    }

    pub const fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

fn arguments(check: &TargetCheck, options: BuildOptions) -> Vec<String> {
    let mut args = Vec::new();
    if check.packages.is_empty() {
        args.push("--workspace".into());
    } else {
        for package in &check.packages {
            args.extend(["--package".into(), package.clone()]);
        }
    }
    args.push(check.artifacts.flag().into());
    if check.all_features {
        args.push("--all-features".into());
    }
    if check.no_default_features {
        args.push("--no-default-features".into());
    }
    for feature in &check.features {
        args.extend(["--features".into(), feature.clone()]);
    }
    if check.profile != "dev" {
        args.extend(["--profile".into(), check.profile.clone()]);
    }
    if options.locked {
        args.push("--locked".into());
    }
    if let Some(target) = &check.target {
        args.extend(["--target".into(), target.clone()]);
    } else if options.host_target {
        args.extend(["--target".into(), "host-tuple".into()]);
    }
    args.push("--".into());
    args.extend(CLIPPY_LINT_ARGS.iter().map(|arg| (*arg).to_owned()));
    args
}

impl Check for TargetChecks {
    fn label(&self) -> &'static str {
        "targets"
    }

    fn cmd(&self) -> String {
        self.commands
            .iter()
            .map(|args| display(args))
            .collect::<Vec<_>>()
            .join("\n  $ ")
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        let mut status = TaskStatus::Pass;
        let mut output = Vec::new();
        for args in &self.commands {
            let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
            let outcome = cargo_outcome(runner, "clippy", &borrowed);
            if !outcome.passed() {
                status = TaskStatus::Fail;
            }
            output.push(format!("{}\n{}", display(args), outcome.output));
        }
        CheckOutcome {
            status,
            output: output.join("\n"),
        }
    }

    fn chain_position(&self) -> Option<u8> {
        Some(5)
    }
}

fn display(args: &[String]) -> String {
    let quoted: Vec<String> = args
        .iter()
        .map(|arg| {
            if arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-.:/=".contains(c))
            {
                arg.clone()
            } else {
                format!("{arg:?}")
            }
        })
        .collect();
    format!("cargo clippy {}", quoted.join(" "))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::config::Artifacts;

    #[test]
    fn duplicate_checks_and_the_enabled_default_are_not_repeated() {
        let release = TargetCheck {
            profile: "release".into(),
            ..TargetCheck::default()
        };
        let config = Config {
            target_checks: vec![TargetCheck::default(), release.clone(), release],
            ..Config::default()
        };
        assert_eq!(TargetChecks::new(&config, true).commands.len(), 1);
        assert_eq!(TargetChecks::new(&config, false).commands.len(), 2);
    }

    #[test]
    fn embedded_selection_overrides_only_its_own_target() {
        let check = TargetCheck {
            target: Some("thumbv8m.main-none-eabihf".into()),
            profile: "release".into(),
            artifacts: Artifacts::Lib,
            packages: vec!["logic".into()],
            all_features: false,
            no_default_features: true,
            features: vec!["motor".into()],
        };
        let options = BuildOptions {
            locked: true,
            host_target: true,
        };
        let args = arguments(&check, options);
        let separator = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(
            &args[..separator],
            [
                "--package",
                "logic",
                "--lib",
                "--no-default-features",
                "--features",
                "motor",
                "--profile",
                "release",
                "--locked",
                "--target",
                "thumbv8m.main-none-eabihf"
            ]
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg == "host-tuple" || arg == "--all-features")
        );
        assert!(
            arguments(&TargetCheck::default(), options)
                .iter()
                .any(|arg| arg == "host-tuple")
        );
    }

    struct RecordingRunner(std::sync::Mutex<Vec<Vec<String>>>);

    impl Runner for RecordingRunner {
        fn spawn(
            &self,
            sub: &str,
            args: &[&str],
            _envs: &[(&str, &str)],
        ) -> std::io::Result<super::super::runner::SpawnResult> {
            assert_eq!(sub, "clippy");
            let mut calls = self.0.lock().unwrap();
            calls.push(args.iter().map(|arg| (*arg).to_string()).collect());
            Ok(super::super::runner::SpawnResult {
                success: calls.len() != 1,
                stdout: Vec::new(),
                stderr: b"target diagnostic".to_vec(),
            })
        }
    }

    #[test]
    fn failure_is_retained_while_later_profiles_are_still_checked() {
        let config = Config {
            target_checks: vec![
                TargetCheck::default(),
                TargetCheck {
                    profile: "release".into(),
                    ..TargetCheck::default()
                },
            ],
            ..Config::default()
        };
        let check = TargetChecks::new(&config, false);
        let runner = RecordingRunner(std::sync::Mutex::new(Vec::new()));
        let outcome = check.run(&runner);
        assert!(outcome.failed());
        assert_eq!(runner.0.lock().unwrap().len(), 2);
        assert!(outcome.output.contains("--profile release"));
        assert!(outcome.output.contains("target diagnostic"));
    }
}
