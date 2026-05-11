// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![allow(clippy::unwrap_used)]

use assert_cmd::prelude::*;
use assert_fs::{TempDir, prelude::*};
use indoc::indoc;
use std::process::Command;

// ── Test fixtures ───────────────────────────────────────────────────────────

/// Minimal `Cargo.toml` for the dummy project.
const CARGO_TOML: &str = indoc! {r#"
[package]
name = "dummy_project"
version = "0.1.0"
edition = "2024"
"#};

/// Source that matches `rustfmt` output exactly (no leading/trailing blanks).
const FORMATTED_MAIN_RS: &str = indoc! {r#"
fn main() {
    println!("Hello!");
}
"#};

/// Valid Rust that compiles but fails `cargo fmt --check`.
const UNFORMATTED_MAIN_RS: &str = indoc! {r#"
fn main(){println!("Hello!");}
"#};

/// Invalid Rust that fails `cargo check`.
const BROKEN_MAIN_RS: &str = indoc! {r#"
fn main() {
    let x: i32 = "not a number";
}
"#};

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Scaffolds a temporary Cargo project with properly formatted source code.
fn dummy_cargo_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    dir.child("Cargo.toml").write_str(CARGO_TOML).unwrap();
    dir.child("src/main.rs")
        .write_str(FORMATTED_MAIN_RS)
        .unwrap();
    dir
}

/// Returns a [`Command`] for the `lockpick` binary with the external-tool
/// checks pre-skipped. Most tests exercise the orchestration layer and
/// do not care about coverage / machete / audit, which would also require
/// the respective cargo subcommands to be installed in the test
/// environment.
///
/// When stderr is piped (not a TTY), lockpick falls back to a plain writer
/// so that summaries and sections are still captured in `Output::stderr`.
fn lockpick() -> Command {
    let mut cmd = Command::cargo_bin("lockpick").unwrap();
    cmd.args(["--skip", "coverage", "--skip", "machete", "--skip", "audit"]);
    cmd
}

/// Returns a [`Command`] for the `lockpick` binary with NO default flags.
/// Use this when testing coverage-related behavior specifically.
fn lockpick_raw() -> Command {
    Command::cargo_bin("lockpick").unwrap()
}

/// Extracts captured stderr from a finished process as a UTF-8 string.
fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// All checks pass on a correctly formatted project (coverage skipped).
#[test]
fn succeeds_on_valid_project() {
    let project = dummy_cargo_project();

    lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .success();
}

/// `cargo fmt --check` detects bad formatting and causes a failure.
#[test]
fn fails_on_unformatted_code() {
    let project = dummy_cargo_project();
    project
        .child("src/main.rs")
        .write_str(UNFORMATTED_MAIN_RS)
        .unwrap();

    lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .failure();
}

/// `--skip fmt` bypasses the formatting check and removes `fmt` from the output entirely.
#[test]
fn skip_fmt_ignores_formatting() {
    let project = dummy_cargo_project();
    project
        .child("src/main.rs")
        .write_str(UNFORMATTED_MAIN_RS)
        .unwrap();

    let output = lockpick()
        .current_dir(project.path())
        .args(["--skip", "fmt"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        !stderr.contains("fmt"),
        "expected no mention of 'fmt' in output, got:\n{stderr}"
    );
}

/// `-v` shows detailed PASS sections for every check that succeeded.
#[test]
fn verbose_shows_pass_sections() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .arg("-v")
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    for section in [
        "CHECK OUTPUT",
        "CLIPPY OUTPUT",
        "FMT OUTPUT",
        "TEST OUTPUT",
        "DOC OUTPUT",
    ] {
        assert!(
            stderr.contains(section),
            "expected '{section}' in verbose output, got:\n{stderr}"
        );
    }
}

/// With `-v`, PASS sections (`✔`) appear before FAIL sections (`✖`).
#[test]
fn verbose_pass_sections_appear_before_fail() {
    let project = dummy_cargo_project();
    project
        .child("src/main.rs")
        .write_str(UNFORMATTED_MAIN_RS)
        .unwrap();

    let output = lockpick()
        .current_dir(project.path())
        .arg("-v")
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure();
    let last_pass = stderr.rfind("OUTPUT").expect("no PASS section found");
    let first_fail = stderr.find("ERRORS").expect("no FAIL section found");
    assert!(
        last_pass < first_fail,
        "expected all PASS sections before FAIL sections:\n{stderr}"
    );
}

/// `--help` prints usage information and exits with code 0.
#[test]
fn help_flag_exits_successfully() {
    lockpick_raw()
        .arg("--help")
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .success();
}

/// Removed `--coverage` and `--min-coverage` flags are rejected with exit 2.
#[test]
fn removed_coverage_flag_is_rejected() {
    lockpick_raw()
        .arg("--coverage")
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .failure()
        .code(2);
}

/// `--skip test` implicitly skips coverage too, so no `cargo-llvm-cov`
/// is required and no `coverage` line appears in the output.
#[test]
fn skip_test_implies_skip_coverage() {
    let project = dummy_cargo_project();

    let output = lockpick_raw()
        .current_dir(project.path())
        .args(["--skip", "test", "--skip", "machete", "--skip", "audit"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        !stderr.contains("coverage"),
        "expected no coverage section when test is skipped, got:\n{stderr}"
    );
}

/// A project that fails `cargo check` causes remaining checks to be skipped.
#[test]
fn check_failure_skips_remaining_checks() {
    let project = dummy_cargo_project();
    project
        .child("src/main.rs")
        .write_str(BROKEN_MAIN_RS)
        .unwrap();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure();

    assert!(
        stderr.contains("FAIL"),
        "expected check to FAIL, got:\n{stderr}"
    );
    for label in ["clippy", "fmt", "test"] {
        assert!(
            stderr.contains(&format!("{label:<8} SKIP")),
            "expected '{label}' to be SKIP, got:\n{stderr}"
        );
    }
}

/// Skipping every check succeeds immediately and logs an informational message.
#[test]
fn skipping_all_checks_succeeds_with_info() {
    let output = lockpick_raw()
        .args([
            "--skip", "check", "--skip", "clippy", "--skip", "fmt", "--skip", "test", "--skip",
            "doc-test", "--skip", "doc", "--skip", "machete", "--skip", "audit", "--skip",
            "license", "--skip", "coverage",
        ])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("All checks disabled, nothing to run"),
        "expected informational message, got:\n{stderr}"
    );
}
