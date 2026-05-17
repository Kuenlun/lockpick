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

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Concatenation of stdout and stderr for "must not appear anywhere"
/// negative assertions, which would otherwise rely on knowing the exact
/// stream each piece of output lands on.
fn combined_text(output: &std::process::Output) -> String {
    let mut out = stdout_text(output);
    out.push_str(&stderr_text(output));
    out
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

    let combined = combined_text(&output);
    output.assert().success();
    assert!(
        !combined.contains("fmt"),
        "expected no mention of 'fmt' on any stream, got:\n{combined}"
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

    let stdout = stdout_text(&output);
    output.assert().success();
    for section in [
        "CHECK OUTPUT",
        "CLIPPY OUTPUT",
        "FMT OUTPUT",
        "TEST OUTPUT",
        "DOC OUTPUT",
    ] {
        assert!(
            stdout.contains(section),
            "expected '{section}' on the report stream, got:\n{stdout}"
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

    let stdout = stdout_text(&output);
    output.assert().failure();
    // Match the unicode banner markers so the assertion is not fooled by
    // "OUTPUT"/"ERRORS" appearing inside a captured cargo message.
    let last_pass = stdout.rfind(" ✔ ").expect("no PASS section found");
    let first_fail = stdout.find(" ✖ ").expect("no FAIL section found");
    assert!(
        last_pass < first_fail,
        "expected all PASS sections before FAIL sections:\n{stdout}"
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

/// Long help must document the Cargo.toml schema so users discover the
/// `[*.metadata.lockpick]` knobs without leaving the terminal.
#[test]
fn long_help_exposes_cargo_metadata_schema() {
    let output = lockpick_raw()
        .arg("--help")
        .output()
        .expect("failed to execute lockpick");

    let stdout = stdout_text(&output);
    output.assert().success();
    for needle in [
        "Configuration:",
        "[workspace.metadata.lockpick]",
        "skip = [",
        "license-header",
        "[workspace.metadata.lockpick.coverage]",
    ] {
        assert!(
            stdout.contains(needle),
            "expected `{needle}` in --help, got:\n{stdout}"
        );
    }
}

/// Long help must surface a copy-paste recipe and the only environment
/// lever the CLI surface cannot express. Pinned together because they
/// landed in the same QA pass (I-3) and share the `after_long_help`
/// block in `cli.rs`.
#[test]
fn long_help_documents_examples_and_no_color_environment() {
    let output = lockpick_raw()
        .arg("--help")
        .output()
        .expect("failed to execute lockpick");

    let stdout = stdout_text(&output);
    output.assert().success();
    for needle in [
        "Examples:",
        "lockpick --skip coverage",
        "NO_COLOR=1 lockpick",
        "Environment:",
        "NO_COLOR",
        "no-color.org",
    ] {
        assert!(
            stdout.contains(needle),
            "expected `{needle}` in --help, got:\n{stdout}"
        );
    }
}

/// Pin clap's wrapping (I-2). Before `wrap_help` was enabled and the
/// auto-`[possible values: ...]` block was hidden, the `--skip` row
/// rendered at 170 chars and spilled past every reasonable terminal.
/// 100 mirrors clap's no-TTY default plus our `max_term_width(100)` cap,
/// so a regression here would have to land in both `Cargo.toml` and
/// `cli.rs` to slip past.
#[test]
fn help_lines_stay_within_a_sane_width_when_piped() {
    for flag in ["-h", "--help"] {
        let output = lockpick_raw()
            .arg(flag)
            .output()
            .expect("failed to execute lockpick");

        let stdout = stdout_text(&output);
        let widest = stdout.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        output.assert().success();

        assert!(
            widest <= 100,
            "`lockpick {flag}` emitted a line {widest} chars wide (> 100); \
             a regression in clap wrapping would break narrow terminals:\n{stdout}",
        );
    }
}

#[test]
fn skip_from_cargo_metadata_disables_a_check_without_a_cli_flag() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(&cargo_toml_strict(
            "skip_from_meta",
            "\n[package.metadata.lockpick]\nskip = [\"fmt\", \"coverage\", \"machete\", \"audit\"]\n",
        ))
        .unwrap();
    project.child("README.md").write_str("").unwrap();
    // Unformatted body would normally fail the run; the metadata skip
    // must disable `fmt` so the pipeline still passes.
    project
        .child("src/main.rs")
        .write_str(UNFORMATTED_MAIN_RS)
        .unwrap();

    let output = lockpick_raw()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let combined = combined_text(&output);
    output.assert().success();
    assert!(
        !combined.contains("fmt"),
        "expected no mention of 'fmt' once it is skipped via metadata, got:\n{combined}"
    );
}

#[test]
fn skip_from_cargo_metadata_rejects_an_unknown_identifier() {
    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(&cargo_toml_strict(
            "skip_bad",
            "\n[package.metadata.lockpick]\nskip = [\"klippy\"]\n",
        ))
        .unwrap();
    project.child("README.md").write_str("").unwrap();
    project
        .child("src/main.rs")
        .write_str(FORMATTED_MAIN_RS)
        .unwrap();

    // A bad value in `skip = [...]` makes the whole config fall back to
    // defaults with a warning, instead of aborting. The run keeps going,
    // but the warning must echo the offending value so the user can
    // pinpoint what to fix in their Cargo.toml.
    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("klippy"),
        "expected the bad value in the warning, got:\n{stderr}"
    );
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
    let combined = combined_text(&output);
    output.assert().success();
    // The inert-skip warning is a diag, so it lives on stderr.
    assert!(
        stderr.contains("--skip test implies coverage will be skipped"),
        "expected note about implicit coverage skip, got:\n{stderr}"
    );
    // Coverage status lines live on the report stream when they fire.
    // Assert their absence across both streams so the test cannot pass
    // just because the line drifted from one stream to the other.
    for tag in ["coverage PASS", "coverage FAIL", "coverage SKIP"] {
        assert!(
            !combined.contains(tag),
            "expected no '{tag}' status line on any stream, got:\n{combined}"
        );
    }
}

#[test]
fn skip_doc_test_notes_no_op_on_bin_only_workspace() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .args(["--skip", "doc-test"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("--skip doc-test has no effect"),
        "expected inert-skip note for doc-test, got:\n{stderr}"
    );
}

#[test]
fn skip_license_notes_no_op_when_no_header_configured() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .args(["--skip", "license"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().success();
    assert!(
        stderr.contains("--skip license has no effect"),
        "expected inert-skip note for license, got:\n{stderr}"
    );
}

#[test]
fn check_failure_skips_chain_tail_but_independent_still_runs() {
    let project = dummy_cargo_project();
    project
        .child("src/main.rs")
        .write_str(BROKEN_MAIN_RS)
        .unwrap();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stdout = stdout_text(&output);
    output.assert().failure();

    assert!(
        stdout.contains("FAIL"),
        "expected check to FAIL, got:\n{stdout}"
    );
    // `10` mirrors `reporter::LABEL_WIDTH`. The chain — everything that
    // would have to compile — is skipped behind a failing `check`.
    for label in ["clippy", "test", "doc"] {
        assert!(
            stdout.contains(&format!("{label:<10} SKIP")),
            "expected chain check '{label}' to be SKIP, got:\n{stdout}"
        );
    }
    // `fmt` is independent of the build — it runs in parallel with the
    // chain and must not be dragged down by a compile failure.
    assert!(
        stdout.contains(&format!("{label:<10} PASS", label = "fmt")),
        "expected independent 'fmt' to PASS regardless of compile failure, got:\n{stdout}"
    );
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

    let stdout = stdout_text(&output);
    output.assert().success();
    assert!(
        stdout.contains("checks passed"),
        "expected success footer on the report stream, got:\n{stdout}"
    );
}

/// Pins the UNIX stdout/stderr split: the report (per-check status
/// table + final summary) lands on stdout so `lockpick > report.txt`
/// captures a useful file, and a quiet successful run leaves stderr
/// empty so callers can probe it for genuine diagnostics.
#[test]
fn report_lands_on_stdout_and_stderr_stays_quiet_on_clean_success() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let stdout = stdout_text(&output);
    let stderr = stderr_text(&output);
    output.assert().success();

    assert!(
        stdout.contains("checks passed"),
        "summary missing from stdout, `lockpick > report.txt` would be empty:\n{stdout}"
    );
    for label in ["check", "clippy", "fmt", "test", "doc"] {
        assert!(
            stdout.contains(&format!("{label:<10} PASS")),
            "status line for '{label}' missing from stdout:\n{stdout}"
        );
    }
    assert!(
        stderr.is_empty(),
        "stderr must stay quiet on a clean non-verbose run, got:\n{stderr}"
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

    let stdout = stdout_text(&output);
    output.assert().failure();
    assert!(
        stdout.contains("Failed:") && stdout.contains("fmt"),
        "expected failure footer mentioning fmt on the report stream, got:\n{stdout}"
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

    let stdout = stdout_text(&output);
    output.assert().success();
    assert!(
        stdout.contains("license") && stdout.contains("PASS"),
        "expected license PASS line on the report stream, got:\n{stdout}"
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

    let stdout = stdout_text(&output);
    output.assert().failure();
    assert!(
        stdout.contains("license") && stdout.contains("FAIL"),
        "expected license FAIL line on the report stream, got:\n{stdout}"
    );
    assert!(
        stdout.contains("main.rs"),
        "expected offending file path on the report stream, got:\n{stdout}"
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

    let stdout = stdout_text(&output);
    output.assert().success();
    assert!(
        stdout.contains("coverage") && stdout.contains("PASS"),
        "expected coverage PASS on the report stream, got:\n{stdout}"
    );
}

#[test]
fn license_header_silently_skipped_when_not_configured() {
    let project = dummy_cargo_project();

    let output = lockpick()
        .current_dir(project.path())
        .output()
        .expect("failed to execute lockpick");

    let combined = combined_text(&output);
    output.assert().success();
    assert!(
        !combined.contains("license"),
        "expected no license check on any stream without config, got:\n{combined}"
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

    let stdout = stdout_text(&output);
    output.assert().success();
    assert!(
        stdout.contains("coverage") && stdout.contains("PASS"),
        "expected coverage PASS via shim on the report stream, got:\n{stdout}"
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

    let stdout = stdout_text(&output);
    output.assert().failure();
    // Both the status line and the FAIL section land on the report
    // stream, so either signal proves the failure surfaced to the user.
    assert!(
        stdout.contains("malformed llvm-cov JSON") || stdout.contains("FAIL"),
        "expected malformed JSON failure on the report stream, got:\n{stdout}"
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
    // Drip-feed regression: a single error must cover every missing
    // tool, with one combined `cargo install` line and one combined
    // `--skip` escape hatch.
    for binary in ["cargo-llvm-cov", "cargo-machete", "cargo-audit"] {
        assert!(
            stderr.contains(binary),
            "expected `{binary}` in error message, got:\n{stderr}"
        );
    }
    assert!(
        stderr.contains("cargo install cargo-llvm-cov cargo-machete cargo-audit"),
        "expected combined install hint, got:\n{stderr}"
    );
    assert!(
        stderr.contains("lockpick --skip coverage --skip machete --skip audit"),
        "expected combined skip hint, got:\n{stderr}"
    );
}

#[test]
fn skipping_all_checks_exits_two_as_a_misconfiguration() {
    // CI guard: a merge gate that ran nothing must never read as
    // green, so disabling every phase via `--skip` is a usage error
    // (exit 2), not success. Equivalent in spirit to pytest's exit 5
    // ("no tests collected").
    let output = lockpick_raw()
        .args([
            "--skip", "check", "--skip", "clippy", "--skip", "fmt", "--skip", "test", "--skip",
            "doc-test", "--skip", "doc", "--skip", "machete", "--skip", "audit", "--skip",
            "license", "--skip", "coverage",
        ])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure().code(2);
    assert!(
        stderr.contains("all checks skipped"),
        "expected misconfiguration error, got:\n{stderr}"
    );
}

/// Install a `cargo-audit` shim at `shim_dir` and return a `PATH` with
/// it prepended. The shim records its cwd (via `pwd -P`, which always
/// queries the kernel) to `LOCKPICK_TEST_AUDIT_CWD_FILE` and exits 0.
#[cfg(unix)]
fn install_cargo_audit_shim(shim_dir: &TempDir) -> String {
    use std::os::unix::fs::PermissionsExt;

    let shim_src = indoc! {r#"#!/bin/sh
        # Cargo plugins receive the plugin name as $1; strip it.
        if [ "$1" = "audit" ]; then shift; fi
        pwd -P > "$LOCKPICK_TEST_AUDIT_CWD_FILE"
    "#};
    let shim_path = shim_dir.child("cargo-audit");
    shim_path.write_str(shim_src).unwrap();
    let mut perms = std::fs::metadata(shim_path.path()).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(shim_path.path(), perms).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", shim_dir.path().display(), original_path)
}

/// Install a `rustc` shim that fakes a stable banner for `--version`
/// (so `is_nightly()` reports stable) and execs the real rustc, whose
/// absolute path is read from `$LOCKPICK_TEST_REAL_RUSTC`, for every
/// other invocation. Returns a `PATH` with `shim_dir` prepended, matching
/// the convention of [`install_cargo_llvm_cov_shim`].
#[cfg(unix)]
fn install_stable_rustc_shim(shim_dir: &TempDir) -> String {
    use std::os::unix::fs::PermissionsExt;

    let shim_src = indoc! {r#"#!/bin/sh
        if [ "$1" = "--version" ]; then
            echo "rustc 1.85.0 (4d91de4e4 2025-02-17)"
            exit 0
        fi
        exec "$LOCKPICK_TEST_REAL_RUSTC" "$@"
    "#};
    let shim_path = shim_dir.child("rustc");
    shim_path.write_str(shim_src).unwrap();
    let mut perms = std::fs::metadata(shim_path.path()).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(shim_path.path(), perms).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", shim_dir.path().display(), original_path)
}

#[cfg(unix)]
#[test]
fn coverage_branches_on_stable_exits_with_four_and_actionable_hint() {
    // Triggers the `BranchesRequireNightly` arm of `dispatch` end-to-end:
    // a project that opts into `coverage.branches` plus a stable rustc
    // banner must short-circuit before any check spawns. The shim only
    // lies about `rustc --version`; cargo's own rustc calls bypass it
    // through `$RUSTC`, so `cargo metadata` keeps working.
    let shim_dir = TempDir::new().unwrap();
    install_stable_rustc_shim(&shim_dir);
    let new_path = install_cargo_llvm_cov_shim(&shim_dir);

    let real_rustc = Command::new("which")
        .arg("rustc")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .expect("real rustc must be on PATH for the shim passthrough");

    let project = TempDir::new().unwrap();
    project
        .child("Cargo.toml")
        .write_str(&cargo_toml_strict(
            "branches_stable",
            "[package.metadata.lockpick.coverage]\nbranches = 80\n",
        ))
        .unwrap();
    project.child("README.md").write_str("").unwrap();
    project
        .child("src/main.rs")
        .write_str(FORMATTED_MAIN_RS)
        .unwrap();

    let output = lockpick_raw()
        .current_dir(project.path())
        .env("PATH", &new_path)
        .env("RUSTC", &real_rustc)
        .env("LOCKPICK_TEST_REAL_RUSTC", &real_rustc)
        .args(["--skip", "machete", "--skip", "audit"])
        .output()
        .expect("failed to execute lockpick");

    let stderr = stderr_text(&output);
    output.assert().failure().code(4);
    assert!(
        stderr.contains("coverage.branches"),
        "expected the offending key in the error, got:\n{stderr}"
    );
    assert!(
        stderr.contains("nightly"),
        "expected the nightly requirement to be named, got:\n{stderr}"
    );
    assert!(
        stderr.contains("rustup toolchain install nightly"),
        "expected the install hint in the error, got:\n{stderr}"
    );
}

/// Install a `cargo` shim at `shim_dir` that passes through the few
/// read-only subcommands lockpick needs at startup (`metadata`, version
/// banner, `locate-project`) and stalls on every other invocation so
/// the test has time to deliver a signal mid-pipeline. Each stalling
/// invocation `touch`es `$LOCKPICK_TEST_SHIM_READY` before sleeping so
/// the test can wait for the first shim to be in flight instead of
/// sleeping a fixed amount. Returns a `PATH` with `shim_dir` prepended.
#[cfg(unix)]
fn install_stalling_cargo_shim(shim_dir: &TempDir) -> String {
    use std::os::unix::fs::PermissionsExt;

    // `exec` swaps the shell out for `sleep` so the PID lockpick
    // registered receives SIGINT directly. Without it, POSIX sh defers
    // signals until the foreground child returns, so the shim would
    // outlive the signal and the assertion on shutdown time would
    // measure shell semantics, not lockpick's signal forwarding.
    let shim_src = indoc! {r#"#!/bin/sh
        case "$1" in
            metadata|locate-project|--version|-V)
                exec "$LOCKPICK_TEST_REAL_CARGO" "$@"
                ;;
            *)
                : > "$LOCKPICK_TEST_SHIM_READY"
                exec sleep 30
                ;;
        esac
    "#};
    let shim_path = shim_dir.child("cargo");
    shim_path.write_str(shim_src).unwrap();
    let mut perms = std::fs::metadata(shim_path.path()).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(shim_path.path(), perms).unwrap();

    let original_path = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", shim_dir.path().display(), original_path)
}

/// P-7 regression. Before, `kill -INT $lockpick_pid` killed the cargo
/// children (via the terminal's foreground process group) and lockpick
/// interpreted their non-zero exits as ordinary check failures, leaving
/// the user with exit `1` instead of the canonical `128 + SIGINT = 130`.
/// The test stalls every cargo subprocess in a shim, waits for the
/// first stall to begin via a readiness sentinel (no fixed sleep, so
/// the test does not flake on loaded CI runners), sends SIGINT to the
/// lockpick PID only (so any propagation must come from lockpick
/// itself, not from terminal-level group delivery), and asserts both
/// pieces: the exit code is 130 and the run winds down promptly because
/// the handler forwarded the signal to the stalling children.
#[cfg(unix)]
#[test]
fn sigint_mid_pipeline_exits_with_130_after_forwarding_to_children() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let real_cargo = Command::new("which")
        .arg("cargo")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .expect("real cargo must be on PATH for the shim passthrough");

    let shim_dir = TempDir::new().unwrap();
    let ready_file = shim_dir.child("shim_ready");
    let new_path = install_stalling_cargo_shim(&shim_dir);
    let project = dummy_cargo_project();

    let mut child = lockpick_raw()
        .current_dir(project.path())
        .env("PATH", &new_path)
        .env("LOCKPICK_TEST_REAL_CARGO", &real_cargo)
        .env("LOCKPICK_TEST_SHIM_READY", ready_file.path())
        .args(["--skip", "coverage", "--skip", "machete", "--skip", "audit"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn lockpick");

    // Wait for the first stalling shim to announce itself. Polling beats
    // a fixed sleep on slow runners, and the 15s ceiling sits well below
    // the 30s shim sleep so a stuck startup fails loudly here instead of
    // running out the shim's timeout.
    let ready_deadline = Instant::now() + Duration::from_secs(15);
    while !ready_file.path().exists() {
        if Instant::now() > ready_deadline {
            let _ = child.kill();
            panic!(
                "stalling shim never created the readiness sentinel at {:?}",
                ready_file.path(),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let kill_status = Command::new("kill")
        .args(["-s", "INT", &child.id().to_string()])
        .status()
        .expect("failed to spawn kill");
    assert!(kill_status.success(), "kill -s INT lockpick failed");

    // Poll `try_wait` instead of blocking in `wait_with_output`: a
    // regression that orphans the stalling shims would otherwise pin
    // this test to 4 × the shim's `sleep 30` before nextest's slow
    // timeout terminates it. The 10s ceiling fails the assertion
    // promptly while still leaving ample headroom for a loaded runner.
    let started = Instant::now();
    let wait_deadline = started + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() > wait_deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "lockpick did not wind down within 10s after SIGINT, \
                     suggesting children were not signal-forwarded",
                );
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }
    let output = child.wait_with_output().expect("wait failed");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(130),
        "expected exit 130 (128 + SIGINT) for a graceful SIGINT shutdown, \
         got code={code:?} stderr=\n{stderr}",
        code = output.status.code(),
    );
}

#[cfg(unix)]
#[test]
fn audit_runs_from_workspace_root_when_lockpick_invoked_in_a_subdirectory() {
    // B-3 regression. `cargo audit` only opens `./Cargo.lock`; unlike
    // build/clippy/fmt/machete it does not walk the manifest tree. Before
    // the fix, running lockpick from `project/src/` produced a spurious
    // audit failure while every other phase quietly succeeded — a wedge
    // that broke the "one binary, one verdict" contract. The shim records
    // its actual cwd so we can assert it was anchored to the workspace
    // root regardless of where the user invoked lockpick from.
    let shim_dir = TempDir::new().unwrap();
    let new_path = install_cargo_audit_shim(&shim_dir);
    let cwd_record = shim_dir.child("audit-cwd.txt");

    let project = dummy_cargo_project();
    let subdir = project.child("src");

    let output = lockpick_raw()
        .current_dir(subdir.path())
        .env("PATH", &new_path)
        .env("LOCKPICK_TEST_AUDIT_CWD_FILE", cwd_record.path())
        .args(["--skip", "coverage", "--skip", "machete"])
        .output()
        .expect("failed to execute lockpick");

    output.assert().success();

    let recorded =
        std::fs::read_to_string(cwd_record.path()).expect("audit shim did not record its cwd");
    // Canonicalise both sides: on systems where the temp prefix is a
    // symlink (e.g. macOS `/tmp` → `/private/tmp`), the recorded path
    // and the project path resolve to the same target but differ as
    // strings.
    let observed = std::fs::canonicalize(recorded.trim()).unwrap();
    let expected = std::fs::canonicalize(project.path()).unwrap();
    assert_eq!(
        observed, expected,
        "cargo-audit must run from the workspace root, not the invoker's cwd",
    );
}
