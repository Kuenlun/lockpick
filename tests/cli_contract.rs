// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::unwrap_used)]

//! Contract tests around lockpick's CLI surface: argv parsing, the
//! empty-pipeline misconfiguration arm, and the `completions`
//! subcommand. None of these depend on cargo executing checks.

mod common;

use common::{
    FORMATTED_MAIN_RS, TestResult, dummy_cargo_project, run_lockpick, scratch_crate, stderr, stdout,
};

#[test]
fn usage_error_on_unknown_skip_value() -> TestResult {
    // clap maps invalid value-enum input to its canonical exit 2,
    // matching every other usage error. The hint must name the bad
    // value and at least one valid replacement so the user has
    // something concrete to copy.
    let cwd = std::env::current_dir()?;
    let out = run_lockpick(&cwd).args(["--skip", "wat"]).output()?;
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 on unknown skip value, got code={code:?} stderr=\n{err}",
        code = out.status.code(),
        err = stderr(&out),
    );
    let err = stderr(&out);
    assert!(
        err.contains("invalid value 'wat'"),
        "missing offender in stderr:\n{err}"
    );
    assert!(
        err.contains("clippy"),
        "missing suggestion in stderr:\n{err}"
    );
    Ok(())
}

#[test]
fn empty_pipeline_returns_exit_two_with_message() -> TestResult {
    // A merge gate that ran nothing must never read as green. The
    // canonical exit is 2 (clap usage), not 1 (check failed).
    let project = dummy_cargo_project();
    let skip_args: Vec<&str> = [
        "check", "clippy", "test", "doc-test", "fmt", "doc", "machete", "audit", "license",
        "coverage",
    ]
    .iter()
    .flat_map(|v| ["--skip", v])
    .collect();

    let out = run_lockpick(project.path()).args(&skip_args).output()?;
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 on empty pipeline, got code={code:?} stderr=\n{err}",
        code = out.status.code(),
        err = stderr(&out),
    );
    let err = stderr(&out);
    assert!(
        err.contains("all checks skipped; nothing to verify"),
        "missing empty-pipeline message:\n{err}"
    );
    Ok(())
}

#[test]
fn coverage_flag_conflicting_with_cli_skip_exits_two() -> TestResult {
    // `--coverage` demands the gate run; combining it with a skip that
    // disables the gate must be a usage error, not a silent winner.
    let project = dummy_cargo_project();
    for skip in ["coverage", "test"] {
        let out = run_lockpick(project.path())
            .args(["--coverage", "--skip", skip])
            .output()?;
        assert_eq!(
            out.status.code(),
            Some(2),
            "expected exit 2 on --coverage --skip {skip}, got code={code:?} stderr=\n{err}",
            code = out.status.code(),
            err = stderr(&out),
        );
        let err = stderr(&out);
        assert!(
            err.contains("`--coverage` conflicts with skipping") && err.contains(skip),
            "missing conflict message for `{skip}`:\n{err}"
        );
    }
    Ok(())
}

#[test]
fn coverage_flag_conflicting_with_config_skip_exits_two() -> TestResult {
    // The conflict guard runs after config skips merge into the CLI
    // view, so a `skip = ["test"]` from Cargo.toml contradicts
    // `--coverage` just like a CLI flag would.
    let project = scratch_crate(
        "config_conflict",
        "[package.metadata.lockpick]\nskip = [\"test\"]\n",
        &[("src/main.rs", FORMATTED_MAIN_RS)],
    );
    let out = run_lockpick(project.path()).arg("--coverage").output()?;
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 on --coverage vs config skip, got code={code:?} stderr=\n{err}",
        code = out.status.code(),
        err = stderr(&out),
    );
    let err = stderr(&out);
    assert!(
        err.contains("config `skip` list"),
        "conflict message should point at the config skip list:\n{err}"
    );
    Ok(())
}

#[test]
fn completions_emit_shell_script_to_stdout() -> TestResult {
    // The script must be self-describing (mentions the binary name).
    // For fish, also assert on the internal `__fish_lockpick_…` symbol
    // so a future renaming of the binary or the clap_complete output
    // format trips the test loudly.
    let cwd = std::env::current_dir()?;
    for shell in ["fish", "bash", "zsh"] {
        let out = run_lockpick(&cwd).args(["completions", shell]).output()?;
        assert_eq!(
            out.status.code(),
            Some(0),
            "completions {shell} expected exit 0, got code={code:?} stderr=\n{err}",
            code = out.status.code(),
            err = stderr(&out),
        );
        let script = stdout(&out);
        assert!(
            !script.is_empty(),
            "completions {shell} produced empty stdout"
        );
        assert!(
            script.contains("lockpick"),
            "completions {shell} script omits the binary name:\n{script}"
        );
        if shell == "fish" {
            assert!(
                script.contains("__fish_lockpick_global_optspecs"),
                "fish completion missing internal symbol:\n{script}"
            );
        }
    }
    Ok(())
}
