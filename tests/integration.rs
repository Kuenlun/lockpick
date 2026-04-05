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

/// Returns a [`Command`] for the `lockpick` binary.
///
/// When stderr is piped (not a TTY), lockpick falls back to a plain writer
/// so that summaries and sections are still captured in `Output::stderr`.
fn lockpick() -> Command {
    Command::cargo_bin("lockpick").unwrap()
}

/// Extracts captured stderr from a finished process as a UTF-8 string.
fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ── Tests ───────────────────────────────────────────────────────────────────

/// All checks pass on a correctly formatted project.
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
    for section in ["CHECK OUTPUT", "CLIPPY OUTPUT", "FMT OUTPUT", "TEST OUTPUT"] {
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
    lockpick()
        .arg("--help")
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .success();
}

/// `--coverage` combined with `--skip test` is rejected with exit code 2.
#[test]
fn coverage_and_skip_test_are_mutually_exclusive() {
    lockpick()
        .args(["--coverage", "--skip", "test"])
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .failure()
        .code(2);
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
    let output = lockpick()
        .args([
            "--skip", "check", "--skip", "clippy", "--skip", "fmt", "--skip", "test", "-vv",
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

/// `--coverage` adds a coverage gate that succeeds when line coverage meets the threshold.
#[test]
#[cfg_attr(coverage, ignore = "avoid nested cargo-llvm-cov invocations")]
fn coverage_flag_adds_coverage_gate() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .args(["-c", "--min-coverage", "0"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("coverage"),
        "expected coverage summary line, got:\n{stderr}"
    );
}
