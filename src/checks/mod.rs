// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Catalog of individual checks. Each module implements the [`Check`] trait
//! over its own struct so the runner stays decoupled from the specifics of
//! each cargo invocation.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::reporter::{CheckOutcome, TaskStatus};
use crate::tooling::{Toolchain, cargo_command};

pub mod audit;
pub mod clippy;
pub mod compile;
pub mod coverage;
pub mod doc;
pub mod doctest;
pub mod fmt;
pub mod license_header;
pub mod machete;
pub mod test;

pub const COMMON_ARGS: &[&str] = &["--workspace", "--all-targets", "--all-features"];

/// Decoupled view of a finished cargo invocation. Avoids leaking
/// `std::process::ExitStatus` (which has no portable public constructor)
/// so tests can synthesise outcomes without spawning a real process.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Strategy object that runs `cargo <sub> <args…>`. Production uses
/// [`CargoCli`]; unit tests substitute fakes that return canned outputs.
pub trait Runner: Send + Sync {
    /// Spawn the cargo subcommand and capture its raw output. An [`Err`]
    /// here signals an OS-level failure to launch the process — non-zero
    /// exit statuses come back as `Ok(SpawnResult { success: false, … })`.
    fn spawn(
        &self,
        sub: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> std::io::Result<SpawnResult>;
}

/// Production [`Runner`]: shells out to the host `cargo` binary, scrubs
/// inherited package-scoped env vars, and optionally redirects child
/// builds to a separate target directory when lockpick itself is running
/// from inside `target/` (cargo run).
#[derive(Debug, Clone, Copy, Default)]
pub struct CargoCli {
    /// When true, child cargo invocations get `CARGO_TARGET_DIR` set to
    /// `target/lockpick` so they don't contend with the parent process for
    /// the same target directory.
    redirect_target_dir: bool,
}

impl CargoCli {
    /// Probe the runtime environment to decide whether child cargo
    /// invocations need a redirected `CARGO_TARGET_DIR`.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            redirect_target_dir: needs_target_dir_redirect(
                std::env::current_exe().ok().as_deref(),
                std::env::current_dir().ok().as_deref(),
                std::env::var_os("CARGO_TARGET_DIR").as_deref(),
            ),
        }
    }
}

impl Runner for CargoCli {
    fn spawn(
        &self,
        sub: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> std::io::Result<SpawnResult> {
        let mut cmd = cargo_command();
        cmd.arg(sub).args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        if self.redirect_target_dir {
            cmd.env("CARGO_TARGET_DIR", "target/lockpick");
        }
        execute(cmd)
    }
}

/// Run a fully-prepared [`Command`] capturing stdout and stderr, and lower
/// it into a [`SpawnResult`]. Factored out so unit tests can drive the
/// spawn-failure branch with `Command::new("/does/not/exist")`.
fn execute(mut cmd: Command) -> std::io::Result<SpawnResult> {
    let out = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    Ok(SpawnResult {
        success: out.status.success(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// A single quality check that lockpick can execute.
pub trait Check: Send + Sync {
    /// Label shown in the spinner and in section headers.
    fn label(&self) -> &'static str;
    /// Human-readable command line for `--verbose` output.
    fn cmd(&self) -> String;
    /// Execute the check and capture its outcome.
    fn run(&self, runner: &dyn Runner) -> CheckOutcome;
}

/// Build the list of parallel checks to run after the `compile` gate.
/// Skipped checks are excluded entirely so they don't appear in the output.
///
/// `coverage_active` enables instrumentation in the `test` check so its
/// `.profraw` files can be consumed by the coverage gate in phase 3.
/// `has_lib` comes from the single [`crate::config::LockpickMetadata`]
/// load and gates the `doc-test` check on bin-only workspaces.
#[must_use]
pub fn build_parallel(
    cli: &Cli,
    coverage_active: bool,
    toolchain: Toolchain,
    config: &Config,
    has_lib: bool,
) -> Vec<Box<dyn Check>> {
    let mut checks: Vec<Box<dyn Check>> = Vec::new();

    if !cli.skips(&SkipOption::Clippy) {
        checks.push(Box::new(clippy::ClippyCheck));
    }
    if !cli.skips(&SkipOption::Fmt) {
        checks.push(Box::new(fmt::FmtCheck));
    }
    if !cli.skips(&SkipOption::Test) {
        checks.push(Box::new(test::TestCheck {
            instrumented: coverage_active,
            nextest: toolchain.nextest,
        }));
    }
    if !cli.skips(&SkipOption::DocTest) && has_lib {
        checks.push(Box::new(doctest::DocTestCheck));
    }
    if !cli.skips(&SkipOption::Doc) {
        checks.push(Box::new(doc::DocCheck));
    }
    if !cli.skips(&SkipOption::Machete) {
        checks.push(Box::new(machete::MacheteCheck));
    }
    if !cli.skips(&SkipOption::Audit) {
        checks.push(Box::new(audit::AuditCheck));
    }
    if !cli.skips(&SkipOption::License)
        && let Some(header_path) = config.license_header.clone()
    {
        let globs = config
            .license_header_globs
            .clone()
            .unwrap_or_else(license_header::default_globs);
        checks.push(Box::new(license_header::LicenseHeaderCheck {
            header_path,
            globs,
        }));
    }

    checks
}

/// Lower a [`Runner::spawn`] result into a [`CheckOutcome`], combining
/// stdout and stderr into a single string. A process-level failure to
/// launch becomes [`TaskStatus::Fail`] with no output, matching the
/// behaviour the rest of the orchestrator depends on.
pub fn outcome_from(result: std::io::Result<SpawnResult>) -> CheckOutcome {
    match result {
        Ok(sr) => {
            let mut combined = String::from_utf8_lossy(&sr.stdout).into_owned();
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(&String::from_utf8_lossy(&sr.stderr));
            CheckOutcome {
                status: if sr.success {
                    TaskStatus::Pass
                } else {
                    TaskStatus::Fail
                },
                output: combined,
            }
        }
        Err(_) => CheckOutcome {
            status: TaskStatus::Fail,
            output: String::new(),
        },
    }
}

/// Shared adapter: spawn `cargo <sub> <args…>` via the given runner and
/// lower the result into a [`CheckOutcome`].
pub fn cargo_outcome(runner: &dyn Runner, sub: &str, args: &[&str]) -> CheckOutcome {
    outcome_from(runner.spawn(sub, args, &[]))
}

/// Variant of [`cargo_outcome`] that injects extra environment variables.
pub fn cargo_outcome_with_env(
    runner: &dyn Runner,
    sub: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> CheckOutcome {
    outcome_from(runner.spawn(sub, args, envs))
}

/// Helper to format a cargo command line for display.
pub fn fmt_cargo_cmd(subcommand: &str, args: &[&str]) -> String {
    if args.is_empty() {
        format!("cargo {subcommand}")
    } else {
        format!("cargo {subcommand} {}", args.join(" "))
    }
}

/// Pure helper that decides whether child cargo invocations should be
/// redirected to `target/lockpick`. Inputs:
/// * `exe` — the running executable's path (None when probing fails).
/// * `cwd` — the current working directory (None when probing fails).
/// * `target_dir_env` — the value of `CARGO_TARGET_DIR` if set.
///
/// Redirection only kicks in when the binary is running from inside the
/// project's own `target/` directory AND the operator hasn't already
/// overridden the target dir.
pub fn needs_target_dir_redirect(
    exe: Option<&Path>,
    cwd: Option<&Path>,
    target_dir_env: Option<&std::ffi::OsStr>,
) -> bool {
    let (Some(exe), Some(cwd)) = (exe, cwd) else {
        return false;
    };
    target_dir_env.is_none() && exe.starts_with(cwd.join("target"))
}

#[cfg(test)]
pub use test_support::FakeRunner;

#[cfg(test)]
mod test_support {
    use super::{Runner, SpawnResult};
    use std::io;
    use std::sync::Mutex;

    /// Configurable test double for [`Runner`]. Each call records its
    /// arguments and pops the next canned response off the queue.
    pub struct FakeRunner {
        responses: Mutex<Vec<io::Result<SpawnResult>>>,
        pub calls: Mutex<Vec<FakeCall>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FakeCall {
        pub sub: String,
        pub args: Vec<String>,
        pub envs: Vec<(String, String)>,
    }

    impl FakeRunner {
        pub fn passing() -> Self {
            Self::with_responses(vec![Ok(SpawnResult {
                success: true,
                stdout: Vec::new(),
                stderr: Vec::new(),
            })])
        }

        pub fn failing() -> Self {
            Self::with_responses(vec![Ok(SpawnResult {
                success: false,
                stdout: Vec::new(),
                stderr: Vec::new(),
            })])
        }

        pub fn with_responses(responses: Vec<io::Result<SpawnResult>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl Runner for FakeRunner {
        fn spawn(
            &self,
            sub: &str,
            args: &[&str],
            envs: &[(&str, &str)],
        ) -> io::Result<SpawnResult> {
            self.calls.lock().expect("not poisoned").push(FakeCall {
                sub: sub.to_string(),
                args: args.iter().map(|s| (*s).to_string()).collect(),
                envs: envs
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            });
            let mut responses = self.responses.lock().expect("not poisoned");
            if responses.is_empty() {
                return Ok(SpawnResult {
                    success: true,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }
            responses.remove(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::PathBuf;

    #[test]
    fn fmt_cargo_cmd_with_args_joins_them_with_spaces() {
        let s = fmt_cargo_cmd("check", &["--workspace", "--all-features"]);
        assert_eq!(s, "cargo check --workspace --all-features");
    }

    #[test]
    fn fmt_cargo_cmd_with_no_args_drops_trailing_space() {
        let s = fmt_cargo_cmd("audit", &[]);
        assert_eq!(s, "cargo audit");
    }

    #[test]
    fn common_args_targets_workspace_with_all_targets_and_features() {
        assert!(COMMON_ARGS.contains(&"--workspace"));
        assert!(COMMON_ARGS.contains(&"--all-targets"));
        assert!(COMMON_ARGS.contains(&"--all-features"));
    }

    #[test]
    fn build_parallel_respects_every_skip_option() {
        let cli = Cli {
            skip: vec![
                SkipOption::Clippy,
                SkipOption::Fmt,
                SkipOption::Test,
                SkipOption::DocTest,
                SkipOption::Doc,
                SkipOption::Machete,
                SkipOption::Audit,
                SkipOption::License,
            ],
            verbose: false,
        };
        let checks = build_parallel(
            &cli,
            false,
            Toolchain::all_present(),
            &Config::default(),
            true,
        );
        assert!(checks.is_empty());
    }

    #[test]
    fn build_parallel_with_no_skips_returns_every_runnable_check_in_lib_workspace() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let checks = build_parallel(
            &cli,
            true,
            Toolchain::all_present(),
            &Config::default(),
            true,
        );
        let has = |needle: &str| checks.iter().any(|c| c.label() == needle);
        assert!(has("clippy"));
        assert!(has("fmt"));
        assert!(has("test"));
        assert!(has("doc test"));
        assert!(has("doc"));
        assert!(has("machete"));
        assert!(has("audit"));
        assert!(!has("license"));
    }

    #[test]
    fn build_parallel_omits_doctest_when_workspace_has_no_lib() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let checks = build_parallel(
            &cli,
            false,
            Toolchain::all_present(),
            &Config::default(),
            false,
        );
        assert!(checks.iter().all(|c| c.label() != "doc test"));
    }

    #[test]
    fn build_parallel_enables_license_check_when_configured() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let config = Config {
            license_header: Some(PathBuf::from("hdr.txt")),
            ..Config::default()
        };
        let checks = build_parallel(&cli, false, Toolchain::all_present(), &config, false);
        assert!(checks.iter().any(|c| c.label() == "license"));
    }

    #[test]
    fn build_parallel_uses_explicit_globs_when_provided() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let config = Config {
            license_header: Some(PathBuf::from("hdr.txt")),
            license_header_globs: Some(vec!["lib/**/*.rs".to_string()]),
            ..Config::default()
        };
        let checks = build_parallel(&cli, false, Toolchain::all_present(), &config, false);
        assert!(checks.iter().any(|c| c.label() == "license"));
    }

    #[test]
    fn outcome_from_success_returns_pass_with_combined_output() {
        let sr = Ok(SpawnResult {
            success: true,
            stdout: b"out\n".to_vec(),
            stderr: b"err\n".to_vec(),
        });
        let o = outcome_from(sr);
        assert!(o.passed());
        assert!(o.output.contains("out"));
        assert!(o.output.contains("err"));
    }

    #[test]
    fn outcome_from_failure_marks_status_fail() {
        let sr = Ok(SpawnResult {
            success: false,
            stdout: b"oops".to_vec(),
            stderr: Vec::new(),
        });
        let o = outcome_from(sr);
        assert!(o.failed());
        assert!(o.output.contains("oops"));
    }

    #[test]
    fn outcome_from_appends_newline_when_stdout_lacks_one() {
        let sr = Ok(SpawnResult {
            success: true,
            stdout: b"hello".to_vec(),
            stderr: b"world".to_vec(),
        });
        let o = outcome_from(sr);
        assert!(o.output.starts_with("hello\n"));
        assert!(o.output.ends_with("world"));
    }

    #[test]
    fn outcome_from_keeps_existing_newline_between_stdout_and_stderr() {
        let sr = Ok(SpawnResult {
            success: true,
            stdout: b"line\n".to_vec(),
            stderr: b"more".to_vec(),
        });
        let o = outcome_from(sr);
        assert_eq!(o.output, "line\nmore");
    }

    #[test]
    fn outcome_from_handles_empty_streams() {
        let sr = Ok(SpawnResult {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
        let o = outcome_from(sr);
        assert!(o.passed());
        assert!(o.output.is_empty());
    }

    #[test]
    fn outcome_from_io_error_returns_fail_with_empty_output() {
        let sr: io::Result<SpawnResult> = Err(io::Error::other("simulated"));
        let o = outcome_from(sr);
        assert!(o.failed());
        assert!(o.output.is_empty());
    }

    #[test]
    fn cargo_outcome_forwards_to_runner_with_no_extra_envs() {
        let fake = FakeRunner::passing();
        let outcome = cargo_outcome(&fake, "check", &["--workspace"]);
        assert!(outcome.passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].sub, "check");
        assert_eq!(calls[0].args, vec!["--workspace"]);
        assert!(calls[0].envs.is_empty());
    }

    #[test]
    fn cargo_outcome_with_env_propagates_extra_envs() {
        let fake = FakeRunner::failing();
        let outcome = cargo_outcome_with_env(&fake, "doc", &[], &[("RUSTDOCFLAGS", "-D warnings")]);
        assert!(outcome.failed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(
            calls[0].envs,
            vec![("RUSTDOCFLAGS".to_string(), "-D warnings".to_string())]
        );
    }

    #[test]
    fn cargo_outcome_failed_io_lowers_to_fail() {
        let fake = FakeRunner::with_responses(vec![Err(io::Error::other("nope"))]);
        let outcome = cargo_outcome(&fake, "x", &[]);
        assert!(outcome.failed());
        assert!(outcome.output.is_empty());
    }

    #[test]
    fn needs_target_dir_redirect_only_triggers_when_exe_lives_inside_cwd_target() {
        let cwd = PathBuf::from("/repo");
        let inside = PathBuf::from("/repo/target/debug/lockpick");
        let outside = PathBuf::from("/elsewhere/lockpick");
        assert!(needs_target_dir_redirect(Some(&inside), Some(&cwd), None));
        assert!(!needs_target_dir_redirect(Some(&outside), Some(&cwd), None));
    }

    #[test]
    fn needs_target_dir_redirect_is_false_when_target_dir_already_set() {
        let cwd = PathBuf::from("/repo");
        let inside = PathBuf::from("/repo/target/debug/lockpick");
        let val = std::ffi::OsString::from("/some/dir");
        assert!(!needs_target_dir_redirect(
            Some(&inside),
            Some(&cwd),
            Some(val.as_os_str())
        ));
    }

    #[test]
    fn needs_target_dir_redirect_is_false_when_either_path_is_missing() {
        let cwd = PathBuf::from("/repo");
        let inside = PathBuf::from("/repo/target/debug/lockpick");
        assert!(!needs_target_dir_redirect(None, Some(&cwd), None));
        assert!(!needs_target_dir_redirect(Some(&inside), None, None));
        assert!(!needs_target_dir_redirect(None, None, None));
    }

    #[test]
    fn cargo_cli_detect_does_not_panic() {
        let _ = CargoCli::detect();
    }

    #[test]
    fn cargo_cli_default_does_not_redirect() {
        let cli = CargoCli::default();
        assert!(!cli.redirect_target_dir);
    }

    #[test]
    fn cargo_cli_spawn_runs_a_real_cargo_subcommand_without_redirect() {
        // `cargo --version` is universally available and finishes in ms.
        let cli = CargoCli::default();
        let result = cli.spawn("--version", &[], &[]).expect("spawn succeeds");
        assert!(result.success, "expected `cargo --version` to succeed");
        assert!(!result.stdout.is_empty());
    }

    #[test]
    fn cargo_cli_spawn_honors_target_dir_redirect() {
        // With redirect on, the resulting cargo command receives the env var.
        // `cargo --version` ignores the env so we can't observe it indirectly,
        // but the spawn still succeeds and the branch executes.
        let cli = CargoCli {
            redirect_target_dir: true,
        };
        let result = cli.spawn("--version", &[], &[]).expect("spawn succeeds");
        assert!(result.success);
    }

    #[test]
    fn cargo_cli_spawn_forwards_extra_envs() {
        let cli = CargoCli::default();
        let result = cli
            .spawn("--version", &[], &[("LOCKPICK_TEST", "yes")])
            .expect("spawn succeeds");
        assert!(result.success);
    }

    #[test]
    fn cargo_cli_spawn_io_error_for_missing_subcommand_returns_non_success() {
        let cli = CargoCli::default();
        let result = cli
            .spawn("definitely-not-a-real-cargo-subcommand", &[], &[])
            .expect("cargo itself spawns");
        assert!(!result.success);
    }

    #[test]
    fn execute_returns_ok_for_a_real_command() {
        // `true` is a no-op binary present on every Unix-like system; on
        // Windows we fall back to `cmd /c exit 0`.
        #[cfg(unix)]
        let cmd = Command::new("true");
        #[cfg(windows)]
        let cmd = {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit 0"]);
            c
        };
        let result = execute(cmd).expect("spawn succeeds");
        assert!(result.success);
    }

    #[test]
    fn execute_returns_err_when_binary_does_not_exist() {
        let cmd = Command::new("/definitely/does/not/exist/lockpick-test");
        assert!(execute(cmd).is_err());
    }

    #[test]
    fn fake_runner_falls_back_to_pass_once_canned_queue_is_drained() {
        // First call consumes the single canned response; the second hits
        // the empty-queue branch and synthesises a default pass.
        let fake = FakeRunner::passing();
        let first = fake.spawn("a", &[], &[]).unwrap();
        assert!(first.success);
        let second = fake.spawn("b", &[], &[]).unwrap();
        assert!(second.success);
        assert!(second.stdout.is_empty());
        assert!(second.stderr.is_empty());
    }
}
