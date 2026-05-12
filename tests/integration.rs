// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
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
    // The unicode markers are exclusive to the section banners; this
    // avoids matching the words "OUTPUT"/"ERRORS" if they ever appear
    // inside a captured cargo message.
    let last_pass = stderr.rfind(" ✔ ").expect("no PASS section found");
    let first_fail = stderr.find(" ✖ ").expect("no FAIL section found");
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
/// is required, no coverage status line appears in the output, and the
/// user is informed of the implicit skip via an always-visible note.
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
        stderr.contains("--skip test implies coverage will be skipped"),
        "expected note about implicit coverage skip, got:\n{stderr}"
    );
    for tag in ["coverage PASS", "coverage FAIL", "coverage SKIP"] {
        assert!(
            !stderr.contains(tag),
            "expected no '{tag}' status line, got:\n{stderr}"
        );
    }
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

/// An unknown value for `--skip` is rejected by clap with exit code 2.
#[test]
fn unknown_skip_value_is_rejected_with_exit_2() {
    lockpick_raw()
        .args(["--skip", "definitely-not-a-real-check"])
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .failure()
        .code(2);
}

/// `-v` prints the planned cargo invocations as a banner before the spinners.
#[test]
fn verbose_prints_planned_commands_banner() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .arg("-v")
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("$ cargo check"),
        "expected planned command banner, got:\n{stderr}"
    );
    assert!(
        stderr.contains("$ cargo clippy"),
        "expected planned command banner, got:\n{stderr}"
    );
}

/// Successful runs end with an "OK: N/N checks passed" footer.
#[test]
fn footer_reports_success_count() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("checks passed"),
        "expected success footer, got:\n{stderr}"
    );
}

/// Failed runs end with a "Failed: K/N (labels)" footer that lists which checks failed.
#[test]
fn footer_lists_failed_checks() {
    let project = dummy_cargo_project();
    project
        .child("src/main.rs")
        .write_str(UNFORMATTED_MAIN_RS)
        .unwrap();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure();
    assert!(
        stderr.contains("Failed:") && stderr.contains("fmt"),
        "expected failure footer mentioning fmt, got:\n{stderr}"
    );
}

/// A project configured with `[package.metadata.lockpick] license-header = ...`
/// runs the license-header check and succeeds when every source file starts
/// with the configured header bytes.
#[test]
fn license_header_passes_when_files_match() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(indoc! {r#"
            [package]
            name = "header_ok"
            version = "0.1.0"
            edition = "2024"

            [package.metadata.lockpick]
            license-header = ".header.txt"
        "#})
        .unwrap();
    project
        .child(".header.txt")
        .write_str("// (c) Lockpick test\n")
        .unwrap();
    project
        .child("src/main.rs")
        .write_str("// (c) Lockpick test\n\nfn main() {}\n")
        .unwrap();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("license") && stderr.contains("PASS"),
        "expected license PASS line, got:\n{stderr}"
    );
}

/// A source file whose header bytes don't match the canonical file is reported
/// by name in the FAIL section and causes lockpick to exit non-zero.
#[test]
fn license_header_fails_and_names_offenders() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(indoc! {r#"
            [package]
            name = "header_bad"
            version = "0.1.0"
            edition = "2024"

            [package.metadata.lockpick]
            license-header = ".header.txt"
        "#})
        .unwrap();
    project
        .child(".header.txt")
        .write_str("// (c) Lockpick test\n")
        .unwrap();
    // Header missing in this file:
    project
        .child("src/main.rs")
        .write_str("fn main() {}\n")
        .unwrap();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure();
    assert!(
        stderr.contains("license") && stderr.contains("FAIL"),
        "expected license FAIL line, got:\n{stderr}"
    );
    assert!(
        stderr.contains("main.rs"),
        "expected offending file path in output, got:\n{stderr}"
    );
}

/// End-to-end: a lib project with 100% coverage of its own code passes the
/// always-on coverage gate. Skipped under nested `cargo llvm-cov` so it does
/// not interfere with lockpick's own coverage measurement.
#[test]
#[cfg_attr(coverage_nightly, ignore = "avoid nested cargo-llvm-cov invocations")]
fn coverage_passes_end_to_end_on_fully_covered_lib() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(indoc! {r#"
            [package]
            name = "cov_ok"
            version = "0.1.0"
            edition = "2024"
        "#})
        .unwrap();
    project
        .child("src/lib.rs")
        .write_str(indoc! {r"
            pub fn add(a: u32, b: u32) -> u32 {
                a + b
            }

            #[cfg(test)]
            mod tests {
                use super::*;

                #[test]
                fn add_works() {
                    assert_eq!(add(2, 3), 5);
                }
            }
        "})
        .unwrap();

    let output = lockpick_raw()
        .current_dir(project.path())
        .args(["--skip", "machete", "--skip", "audit"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("coverage") && stderr.contains("PASS"),
        "expected coverage PASS, got:\n{stderr}"
    );
}

/// Without `[package.metadata.lockpick] license-header`, the license check is
/// silently absent from the output.
#[test]
fn license_header_silently_skipped_when_not_configured() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        !stderr.contains("license"),
        "expected no license check in output without config, got:\n{stderr}"
    );
}

/// Install a canned `cargo-llvm-cov` shim at `shim_dir` and return a
/// PATH value with the shim prepended. The shim accepts both phases that
/// lockpick drives:
///   * test phase: `cargo llvm-cov --branch --no-report …` → exit 0.
///   * coverage phase: `cargo llvm-cov report --json … --branch` → JSON
///     supplied via the `LOCKPICK_TEST_COV_JSON` env var.
#[cfg(unix)]
fn install_cargo_llvm_cov_shim(shim_dir: &TempDir) -> String {
    use std::os::unix::fs::PermissionsExt;

    let shim_src = indoc! {r#"#!/bin/sh
        # When invoked via cargo plugin convention the first arg is the
        # subcommand name ("llvm-cov"); strip it so `$1` is the subcommand
        # lockpick passes.
        if [ "$1" = "llvm-cov" ]; then shift; fi
        case "$1" in
            report)
                printf '%s' "$LOCKPICK_TEST_COV_JSON"
                ;;
            *)
                : # pretend `cargo llvm-cov --no-report …` succeeded
                ;;
        esac
    "#};
    let shim_path = shim_dir.child("cargo-llvm-cov");
    shim_path.write_str(shim_src).unwrap();
    let mut perms = std::fs::metadata(shim_path.path()).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(shim_path.path(), perms).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", shim_dir.path().display(), original_path)
}

/// Run lockpick's coverage gate end-to-end against a mocked
/// `cargo-llvm-cov` shim. This is the only way to exercise the production
/// binary's `evaluate`/`collect_report` code paths without nesting a real
/// cargo-llvm-cov invocation (which conflicts with the outer one).
#[cfg(unix)]
#[test]
fn coverage_runs_against_mocked_cargo_llvm_cov() {
    let shim_dir = TempDir::new().unwrap();
    let new_path = install_cargo_llvm_cov_shim(&shim_dir);

    let project = dummy_cargo_project();
    let canned_json = r#"{ "data": [{ "files": [{}], "totals": { "functions": { "count": 1, "covered": 1 }, "lines": { "count": 1, "covered": 1 }, "regions": { "count": 1, "covered": 1 }, "branches": { "count": 1, "covered": 1 } } }] }"#;

    let output = lockpick_raw()
        .current_dir(project.path())
        .env("PATH", &new_path)
        .env("LOCKPICK_TEST_COV_JSON", canned_json)
        .args(["--skip", "machete", "--skip", "audit"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("coverage") && stderr.contains("PASS"),
        "expected coverage PASS via shim, got:\n{stderr}"
    );
}

/// Same shim setup, but the shim emits malformed JSON so lockpick's
/// `collect_report` exercises its `map_err` arm and lockpick exits with
/// a coverage failure.
#[cfg(unix)]
#[test]
fn coverage_fails_when_shim_returns_malformed_json() {
    let shim_dir = TempDir::new().unwrap();
    let new_path = install_cargo_llvm_cov_shim(&shim_dir);
    let project = dummy_cargo_project();

    let output = lockpick_raw()
        .current_dir(project.path())
        .env("PATH", &new_path)
        .env("LOCKPICK_TEST_COV_JSON", "definitely not JSON")
        .args(["--skip", "machete", "--skip", "audit"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure();
    assert!(
        stderr.contains("malformed llvm-cov JSON") || stderr.contains("FAIL"),
        "expected malformed JSON failure, got:\n{stderr}"
    );
}

/// Sanitising `PATH` so cargo-llvm-cov / cargo-machete / cargo-audit cannot be
/// located trips the "required tool is not installed" path. lockpick must
/// exit with code 3 and surface the install hint instead of running checks.
/// This is the end-to-end coverage for [`crate::dispatch`]'s `MissingTool`
/// arm — no unit test lives in `main.rs` since the production binary is
/// what executes the match.
#[test]
fn missing_required_tool_exits_with_three_and_prints_install_hint() {
    let project = dummy_cargo_project();

    let output = lockpick_raw()
        .current_dir(project.path())
        .env("PATH", "")
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure().code(3);
    assert!(
        stderr.contains("cargo install "),
        "expected install hint in error message, got:\n{stderr}"
    );
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
