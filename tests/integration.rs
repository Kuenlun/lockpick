// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
// `allow-unwrap-in-tests` only scopes to `#[test]` fns; this file-level
// allow keeps the helpers lint-clean too.
#![allow(clippy::unwrap_used)]

use assert_cmd::prelude::*;
use assert_fs::{TempDir, prelude::*};
use indoc::indoc;
use std::process::Command;

// ── Test fixtures ───────────────────────────────────────────────────────────

/// Manifest with every field `cargo_common_metadata` demands so strict
/// clippy stays quiet on the fixture.
const CARGO_TOML: &str = indoc! {r#"
[package]
name = "dummy_project"
version = "0.1.0"
edition = "2024"
description = "Integration-test fixture for lockpick"
license = "MIT OR Apache-2.0"
repository = "https://example.invalid/dummy"
readme = "README.md"
keywords = ["test"]
categories = ["development-tools"]
"#};

/// Matches `rustfmt` output byte-for-byte.
const FORMATTED_MAIN_RS: &str = indoc! {r#"
fn main() {
    println!("Hello!");
}
"#};

/// Compiles cleanly but fails `cargo fmt --check`.
const UNFORMATTED_MAIN_RS: &str = indoc! {r#"
fn main(){println!("Hello!");}
"#};

/// Fails `cargo check`.
const BROKEN_MAIN_RS: &str = indoc! {r#"
fn main() {
    let x: i32 = "not a number";
}
"#};

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Scaffold a temp Cargo project that passes strict clippy out of the box.
fn dummy_cargo_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    dir.child("Cargo.toml").write_str(CARGO_TOML).unwrap();
    dir.child("README.md").write_str("").unwrap();
    dir.child("src/main.rs")
        .write_str(FORMATTED_MAIN_RS)
        .unwrap();
    dir
}

/// Render a `Cargo.toml` body with the strict-clippy metadata baseline
/// and an optional extra TOML fragment appended at the end.
fn cargo_toml_strict(name: &str, extra: &str) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\
         description = \"Integration-test fixture for lockpick\"\n\
         license = \"MIT OR Apache-2.0\"\n\
         repository = \"https://example.invalid/{name}\"\n\
         readme = \"README.md\"\n\
         keywords = [\"test\"]\n\
         categories = [\"development-tools\"]\n\
         {extra}",
    )
}

/// `lockpick` command with the external-tool checks pre-skipped.
fn lockpick() -> Command {
    let mut cmd = Command::cargo_bin("lockpick").unwrap();
    cmd.args(["--skip", "coverage", "--skip", "machete", "--skip", "audit"]);
    cmd
}

/// `lockpick` command with no implicit `--skip` flags.
fn lockpick_raw() -> Command {
    Command::cargo_bin("lockpick").unwrap()
}

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ── Tests ───────────────────────────────────────────────────────────────────

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
    // Match the unicode banner markers so the assertion is not fooled by
    // "OUTPUT"/"ERRORS" appearing inside a captured cargo message.
    let last_pass = stderr.rfind(" ✔ ").expect("no PASS section found");
    let first_fail = stderr.find(" ✖ ").expect("no FAIL section found");
    assert!(
        last_pass < first_fail,
        "expected all PASS sections before FAIL sections:\n{stderr}"
    );
}

#[test]
fn help_flag_exits_successfully() {
    lockpick_raw()
        .arg("--help")
        .output()
        .expect("failed to execute lockpick")
        .assert()
        .success();
}

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
    // `10` mirrors `reporter::LABEL_WIDTH`.
    for label in ["clippy", "fmt", "test"] {
        assert!(
            stderr.contains(&format!("{label:<10} SKIP")),
            "expected '{label}' to be SKIP, got:\n{stderr}"
        );
    }
}

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

#[test]
fn license_header_passes_when_files_match() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(&cargo_toml_strict(
            "header_ok",
            "\n[package.metadata.lockpick]\nlicense-header = \".header.txt\"\n",
        ))
        .unwrap();
    project.child("README.md").write_str("").unwrap();
    project
        .child(".header.txt")
        .write_str("// (c) Lockpick test\n")
        .unwrap();
    // Non-trivial body keeps strict clippy's `missing_const_for_fn`
    // quiet so the run fails on the license check, not on clippy.
    project
        .child("src/main.rs")
        .write_str("// (c) Lockpick test\n\nfn main() {\n    println!(\"hi\");\n}\n")
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

#[test]
fn license_header_fails_and_names_offenders() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(&cargo_toml_strict(
            "header_bad",
            "\n[package.metadata.lockpick]\nlicense-header = \".header.txt\"\n",
        ))
        .unwrap();
    project.child("README.md").write_str("").unwrap();
    project
        .child(".header.txt")
        .write_str("// (c) Lockpick test\n")
        .unwrap();
    // Header missing; non-trivial body keeps strict clippy quiet so the
    // run fails on the license check rather than on clippy.
    project
        .child("src/main.rs")
        .write_str("fn main() {\n    println!(\"hi\");\n}\n")
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

#[test]
#[cfg_attr(coverage_nightly, ignore = "avoid nested cargo-llvm-cov invocations")]
fn coverage_passes_end_to_end_on_fully_covered_lib() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(&cargo_toml_strict("cov_ok", ""))
        .unwrap();
    project.child("README.md").write_str("").unwrap();
    // `#[must_use]` + `const fn` keep strict clippy
    // (`must_use_candidate`, `missing_const_for_fn`) silent.
    project
        .child("src/lib.rs")
        .write_str(indoc! {r"
            #[must_use]
            pub const fn add(a: u32, b: u32) -> u32 {
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

/// Install a `cargo-llvm-cov` shim at `shim_dir` and return a `PATH`
/// with it prepended. The shim returns JSON from `LOCKPICK_TEST_COV_JSON`
/// for the `report` subcommand and exits 0 otherwise.
#[cfg(unix)]
fn install_cargo_llvm_cov_shim(shim_dir: &TempDir) -> String {
    use std::os::unix::fs::PermissionsExt;

    let shim_src = indoc! {r#"#!/bin/sh
        # Cargo plugins receive the plugin name as $1; strip it.
        if [ "$1" = "llvm-cov" ]; then shift; fi
        case "$1" in
            report) printf '%s' "$LOCKPICK_TEST_COV_JSON" ;;
            *) : ;;
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
