// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Individual checks. Each module implements [`Check`] over its own
//! struct, keeping the runner agnostic of the cargo invocation details.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::cli::{Cli, SkipOption};
use crate::config::Config;
use crate::reporter::{CheckOutcome, TaskStatus};
use crate::tooling::{Tool, Toolchain, cargo_command};

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

/// Captured output of a finished cargo invocation. Synthesizable from
/// fakes, since [`std::process::ExitStatus`] has no public constructor.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Strategy that runs `cargo <sub> <args…>`. Production uses [`CargoCli`].
pub trait Runner: Send + Sync {
    /// Spawn the subcommand and capture its raw output.
    ///
    /// [`Err`] signals an OS-level launch failure; non-zero exits come
    /// back as `Ok(SpawnResult { success: false, … })`.
    fn spawn(
        &self,
        sub: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> std::io::Result<SpawnResult>;
}

/// Production [`Runner`]: shells out to the host `cargo`, scrubs
/// package-scoped env vars, and optionally redirects child builds away
/// from the parent's target directory.
#[derive(Debug, Clone, Copy, Default)]
pub struct CargoCli {
    /// When true, children inherit `CARGO_TARGET_DIR=target/lockpick`.
    redirect_target_dir: bool,
}

impl CargoCli {
    /// Decide whether children need `CARGO_TARGET_DIR` redirected.
    #[cfg_attr(test, allow(dead_code))]
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

/// Run a [`Command`] capturing both streams; lower into a [`SpawnResult`].
fn execute(mut cmd: Command) -> std::io::Result<SpawnResult> {
    let out = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    Ok(SpawnResult {
        success: out.status.success(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// Slot for a check inside the serial chain that competes for
/// `target/.cargo-lock`. Lower values run first; gaps are allowed.
///
/// The chain models the dependency every cargo build subcommand has on
/// the per-`target/` exclusive lock — running two of these in parallel
/// would just block on the lock and noisily print `Blocking waiting for
/// file lock`. See the `## Scheduling` section of the README.
pub mod chain {
    pub const COMPILE: u8 = 0;
    pub const TEST: u8 = 1;
    pub const CLIPPY: u8 = 2;
    pub const DOC: u8 = 3;
    pub const DOCTEST: u8 = 4;
}

/// A single quality check.
pub trait Check: Send + Sync {
    /// Label shown in spinners and section headers.
    fn label(&self) -> &'static str;
    /// Human-readable command line for `--verbose` output.
    fn cmd(&self) -> String;
    /// Execute the check.
    fn run(&self, runner: &dyn Runner) -> CheckOutcome;
    /// Position of this check inside the serial chain that competes
    /// for `target/.cargo-lock`. `None` marks an independent check
    /// safe to run in parallel with everything else (it does not
    /// touch `target/`).
    ///
    /// Canonical positions live in [`chain`]; lower runs first.
    fn chain_position(&self) -> Option<u8>;
}

/// The full schedule of checks that survived CLI/config gating.
///
/// Items keep insertion order so the verbose section list and the
/// final summary stay stable run-to-run. The runner partitions them
/// into two cohorts that Cargo's per-`target/` lock actually allows
/// to overlap: an independent cohort and a serial chain.
pub struct Plan {
    items: Vec<Box<dyn Check>>,
}

impl Plan {
    /// Number of checks scheduled, across both cohorts.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the plan has zero checks to run.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate every check with its insertion index, for display.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &dyn Check)> {
        self.items.iter().enumerate().map(|(i, c)| (i, c.as_ref()))
    }

    /// Checks that do not touch `target/` and so run in parallel with
    /// each other and with the serial chain.
    pub fn independent(&self) -> impl Iterator<Item = (usize, &dyn Check)> {
        self.iter().filter(|(_, c)| c.chain_position().is_none())
    }

    /// Checks that compete for `target/.cargo-lock`, sorted by their
    /// declared chain position so the runner walks them in canonical
    /// order — `compile → test → clippy → doc → doc-test` — regardless
    /// of insertion order.
    pub fn serial_chain(&self) -> impl Iterator<Item = (usize, &dyn Check)> {
        let mut chain: Vec<(u8, usize, &dyn Check)> = self
            .iter()
            .filter_map(|(i, c)| c.chain_position().map(|p| (p, i, c)))
            .collect();
        chain.sort_by_key(|(p, _, _)| *p);
        chain.into_iter().map(|(_, i, c)| (i, c))
    }

    /// Build a [`Plan`] from a hand-rolled vector. Test-only: production
    /// code goes through [`build_plan`] so nothing else can construct a
    /// schedule that bypasses CLI/config gating.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    #[must_use]
    pub fn from_items(items: Vec<Box<dyn Check>>) -> Self {
        Self { items }
    }
}

/// Assemble the [`Plan`] of checks that survived CLI/config gating.
///
/// Insertion order doubles as display order — the verbose section list
/// and the final summary follow it. Execution order inside the serial
/// chain is decoupled and lives in [`Check::chain_position`].
///
/// `coverage_active` instruments the `test` check so its `.profraw`
/// files feed the coverage gate; `has_lib` gates the doc-test check;
/// `branch_coverage` (true on nightly) passes `--branch` to the
/// instrumented test run.
#[must_use]
pub fn build_plan(
    cli: &Cli,
    coverage_active: bool,
    toolchain: &Toolchain,
    config: &Config,
    has_lib: bool,
    branch_coverage: bool,
) -> Plan {
    let mut items: Vec<Box<dyn Check>> = Vec::new();

    if !cli.skips(&SkipOption::Check) {
        items.push(Box::new(compile::CompileCheck));
    }
    if !cli.skips(&SkipOption::Clippy) {
        items.push(Box::new(clippy::ClippyCheck));
    }
    if !cli.skips(&SkipOption::Fmt) {
        items.push(Box::new(fmt::FmtCheck));
    }
    if !cli.skips(&SkipOption::Test) {
        items.push(Box::new(test::TestCheck {
            instrumented: coverage_active,
            nextest: toolchain.has(Tool::Nextest),
            branch_coverage,
        }));
    }
    if !cli.skips(&SkipOption::Doc) {
        items.push(Box::new(doc::DocCheck));
    }
    if !cli.skips(&SkipOption::DocTest) && has_lib {
        items.push(Box::new(doctest::DocTestCheck));
    }
    if !cli.skips(&SkipOption::Machete) {
        items.push(Box::new(machete::MacheteCheck));
    }
    if !cli.skips(&SkipOption::Audit) {
        items.push(Box::new(audit::AuditCheck));
    }
    if !cli.skips(&SkipOption::License)
        && let Some(header_path) = config.license_header.clone()
    {
        let globs = config
            .license_header_globs
            .clone()
            .unwrap_or_else(license_header::default_globs);
        items.push(Box::new(license_header::LicenseHeaderCheck {
            header_path,
            globs,
        }));
    }

    Plan { items }
}

/// Concatenate `stdout` and `stderr`, inserting a newline between them
/// when stdout does not already end with one.
#[must_use]
pub fn combine_streams(stdout: &[u8], stderr: &[u8]) -> String {
    let mut combined = String::from_utf8_lossy(stdout).into_owned();
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&String::from_utf8_lossy(stderr));
    combined
}

/// Lower a [`Runner::spawn`] result into a [`CheckOutcome`]. A launch
/// failure becomes [`TaskStatus::Fail`] with empty output.
pub fn outcome_from(result: std::io::Result<SpawnResult>) -> CheckOutcome {
    match result {
        Ok(sr) => CheckOutcome {
            status: if sr.success {
                TaskStatus::Pass
            } else {
                TaskStatus::Fail
            },
            output: combine_streams(&sr.stdout, &sr.stderr),
        },
        Err(_) => CheckOutcome {
            status: TaskStatus::Fail,
            output: String::new(),
        },
    }
}

/// Spawn `cargo <sub> <args…>` and lower the result into a [`CheckOutcome`].
pub fn cargo_outcome(runner: &dyn Runner, sub: &str, args: &[&str]) -> CheckOutcome {
    outcome_from(runner.spawn(sub, args, &[]))
}

/// Like [`cargo_outcome`] but with extra env vars.
pub fn cargo_outcome_with_env(
    runner: &dyn Runner,
    sub: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> CheckOutcome {
    outcome_from(runner.spawn(sub, args, envs))
}

/// Format a cargo command line for display.
pub fn fmt_cargo_cmd(subcommand: &str, args: &[&str]) -> String {
    if args.is_empty() {
        format!("cargo {subcommand}")
    } else {
        format!("cargo {subcommand} {}", args.join(" "))
    }
}

/// Whether child cargo invocations should redirect their target dir.
///
/// Redirects only when the running binary lives under `cwd/target/` and
/// `CARGO_TARGET_DIR` is unset.
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
#[cfg_attr(coverage_nightly, coverage(off))]
mod test_support {
    use super::{Runner, SpawnResult};
    use std::io;
    use std::sync::Mutex;

    /// Test double for [`Runner`]: records calls and pops canned responses.
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
            assert!(
                !responses.is_empty(),
                "FakeRunner ran out of canned responses (subcommand: {sub})",
            );
            responses.remove(0)
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use std::io;
    use std::path::PathBuf;

    #[test]
    fn build_plan_respects_every_skip_option() {
        let cli = Cli {
            skip: vec![
                SkipOption::Check,
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
        let plan = build_plan(
            &cli,
            false,
            &Toolchain::all_present(),
            &Config::default(),
            true,
            false,
        );
        assert!(plan.is_empty());
        assert_eq!(plan.len(), 0);
    }

    #[test]
    fn build_plan_with_no_skips_returns_every_runnable_check_in_lib_workspace() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let plan = build_plan(
            &cli,
            true,
            &Toolchain::all_present(),
            &Config::default(),
            true,
            true,
        );
        let has = |needle: &str| plan.iter().any(|(_, c)| c.label() == needle);
        assert!(has("check"));
        assert!(has("clippy"));
        assert!(has("fmt"));
        assert!(has("test"));
        assert!(has("doc"));
        assert!(has("doc-test"));
        assert!(has("machete"));
        assert!(has("audit"));
        assert!(!has("license"));
    }

    #[test]
    fn build_plan_display_order_matches_readme_example() {
        // The order returned by `Plan::iter` drives spinners, the verbose
        // section listing and the final summary. Pin it against the
        // example block in `README.md`.
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let config = Config {
            license_header: Some(PathBuf::from("hdr.txt")),
            ..Config::default()
        };
        let plan = build_plan(&cli, true, &Toolchain::all_present(), &config, true, true);
        let labels: Vec<&str> = plan.iter().map(|(_, c)| c.label()).collect();
        assert_eq!(
            labels,
            vec![
                "check", "clippy", "fmt", "test", "doc", "doc-test", "machete", "audit", "license",
            ],
        );
    }

    #[test]
    fn build_plan_omits_compile_when_check_skipped() {
        let cli = Cli {
            skip: vec![SkipOption::Check],
            verbose: false,
        };
        let plan = build_plan(
            &cli,
            false,
            &Toolchain::all_present(),
            &Config::default(),
            false,
            false,
        );
        assert!(plan.iter().all(|(_, c)| c.label() != "check"));
    }

    #[test]
    fn build_plan_omits_doctest_when_workspace_has_no_lib() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let plan = build_plan(
            &cli,
            false,
            &Toolchain::all_present(),
            &Config::default(),
            false,
            false,
        );
        assert!(plan.iter().all(|(_, c)| c.label() != "doc-test"));
    }

    #[test]
    fn build_plan_enables_license_check_when_configured() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let config = Config {
            license_header: Some(PathBuf::from("hdr.txt")),
            ..Config::default()
        };
        let plan = build_plan(
            &cli,
            false,
            &Toolchain::all_present(),
            &config,
            false,
            false,
        );
        assert!(plan.iter().any(|(_, c)| c.label() == "license"));
    }

    #[test]
    fn build_plan_uses_explicit_globs_when_provided() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let config = Config {
            license_header: Some(PathBuf::from("hdr.txt")),
            license_header_globs: Some(vec!["lib/**/*.rs".to_string()]),
            ..Config::default()
        };
        let plan = build_plan(
            &cli,
            false,
            &Toolchain::all_present(),
            &config,
            false,
            false,
        );
        assert!(plan.iter().any(|(_, c)| c.label() == "license"));
    }

    #[test]
    fn build_plan_threads_branch_coverage_into_the_instrumented_test_check() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        // Pin both branches of the new field: nightly→ --branch lives in
        // the test invocation, stable→ it is gone. Pins the runner→checks
        // wiring against a future refactor that might drop the flag.
        for branch_coverage in [false, true] {
            let plan = build_plan(
                &cli,
                true,
                &Toolchain::all_present(),
                &Config::default(),
                false,
                branch_coverage,
            );
            let test_cmd = plan
                .iter()
                .find_map(|(_, c)| (c.label() == "test").then(|| c.cmd()))
                .expect("plan must include a `test` check");
            assert_eq!(
                test_cmd.contains("--branch"),
                branch_coverage,
                "test cmd `{test_cmd}` did not match branch_coverage={branch_coverage}",
            );
        }
    }

    #[test]
    fn build_plan_routes_each_check_into_the_cohort_its_chain_position_demands() {
        let cli = Cli {
            skip: vec![],
            verbose: false,
        };
        let config = Config {
            license_header: Some(PathBuf::from("hdr.txt")),
            ..Config::default()
        };
        let plan = build_plan(&cli, true, &Toolchain::all_present(), &config, true, true);

        // Independent cohort: only checks that do not touch `target/`.
        let independent: Vec<&str> = plan.independent().map(|(_, c)| c.label()).collect();
        assert_eq!(independent, vec!["fmt", "machete", "audit", "license"]);

        // Serial chain in canonical order — pinned by `chain_position`,
        // independent of the order build_plan inserted them in.
        let chain: Vec<&str> = plan.serial_chain().map(|(_, c)| c.label()).collect();
        assert_eq!(chain, vec!["check", "test", "clippy", "doc", "doc-test"]);
    }

    #[test]
    fn plan_iter_preserves_insertion_order_across_both_cohorts() {
        let plan = Plan::from_items(vec![
            Box::new(clippy::ClippyCheck),
            Box::new(fmt::FmtCheck),
            Box::new(audit::AuditCheck),
        ]);
        let labels: Vec<&str> = plan.iter().map(|(_, c)| c.label()).collect();
        assert_eq!(labels, vec!["clippy", "fmt", "audit"]);
        let indices: Vec<usize> = plan.iter().map(|(i, _)| i).collect();
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn plan_independent_and_serial_chain_carry_the_original_index() {
        let plan = Plan::from_items(vec![
            Box::new(clippy::ClippyCheck), // 0, chain (CLIPPY = 2)
            Box::new(fmt::FmtCheck),       // 1, independent
            Box::new(audit::AuditCheck),   // 2, independent
            Box::new(doc::DocCheck),       // 3, chain (DOC = 3)
        ]);
        assert_eq!(
            plan.independent().map(|(i, _)| i).collect::<Vec<_>>(),
            vec![1, 2]
        );
        // Chain order follows chain_position, not insertion: clippy (2)
        // before doc (3) regardless of the inserted order.
        assert_eq!(
            plan.serial_chain().map(|(i, _)| i).collect::<Vec<_>>(),
            vec![0, 3]
        );
    }

    #[test]
    fn plan_serial_chain_sorts_by_position_not_insertion_order() {
        // Reverse insertion: doc (3) → clippy (2) → test (1) → check (0).
        // Serial chain must still come back in canonical order.
        let plan = Plan::from_items(vec![
            Box::new(doc::DocCheck),
            Box::new(clippy::ClippyCheck),
            Box::new(test::TestCheck {
                instrumented: false,
                nextest: false,
                branch_coverage: false,
            }),
            Box::new(compile::CompileCheck),
        ]);
        let labels: Vec<&str> = plan.serial_chain().map(|(_, c)| c.label()).collect();
        assert_eq!(labels, vec!["check", "test", "clippy", "doc"]);
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
    fn cargo_cli_spawn_runs_a_real_cargo_subcommand_without_redirect() {
        let cli = CargoCli::default();
        let result = cli.spawn("--version", &[], &[]).expect("spawn succeeds");
        assert!(result.success, "expected `cargo --version` to succeed");
        assert!(!result.stdout.is_empty());
    }

    #[test]
    fn cargo_cli_spawn_honors_target_dir_redirect() {
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
}
