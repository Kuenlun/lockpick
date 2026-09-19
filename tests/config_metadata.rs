// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! `[*.metadata.lockpick]` discovery: package-level on a single-crate
//! project and workspace-level on a multi-crate workspace. The
//! workspace test is also the regression guard for the
//! `workspace_metadata` JSON-key bug (cargo emits `metadata`, not
//! `workspace_metadata`).

mod common;

#[cfg(unix)]
use common::{FORMATTED_MAIN_RS, cargo_toml_strict};
use common::{TestResult, combined, scratch_crate, stdout};

#[cfg(unix)]
#[test]
fn package_metadata_skip_list_is_honored() -> TestResult {
    let (_path_dir, path) = common::sanitized_path()?;
    let project = scratch_crate(
        "skip_via_meta",
        "[package.metadata.lockpick]\nskip = [\"audit\", \"machete\"]\n",
        &[("src/main.rs", FORMATTED_MAIN_RS)],
    );

    let out = common::run_lockpick(project.path())
        .env("PATH", &path)
        .output()?;
    let view = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(0_i32),
        "expected exit 0 with skipped optional checks, got code={code:?}, combined output=\n{view}",
        code = out.status.code(),
    );
    assert!(
        !view.contains("audit"),
        "audit should never appear (skipped via metadata), combined output=\n{view}"
    );
    assert!(
        !view.contains("machete"),
        "machete should never appear (skipped via metadata), combined output=\n{view}"
    );
    let report = stdout(&out);
    assert!(report.contains("OK:"), "missing success summary:\n{report}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn workspace_metadata_skip_list_is_honored() -> TestResult {
    // Regression for the `serde(rename = "metadata")` fix. Cargo's
    // top-level JSON key is `metadata`, not `workspace_metadata`; before
    // the fix the workspace-scoped skip list was silently dropped and
    // this test would attempt to run audit and machete (and fail under
    // the sanitised PATH).
    let (_path_dir, path) = common::sanitized_path()?;
    let workspace = tempfile::tempdir()?;
    let root_toml = "[workspace]\n\
                     members = [\"alpha\", \"beta\"]\n\
                     resolver = \"2\"\n\
                     \n\
                     [workspace.metadata.lockpick]\n\
                     skip = [\"audit\", \"machete\"]\n";
    std::fs::write(workspace.path().join("Cargo.toml"), root_toml)?;

    for name in ["alpha", "beta"] {
        let member = workspace.path().join(name);
        std::fs::create_dir_all(member.join("src"))?;
        std::fs::write(member.join("Cargo.toml"), cargo_toml_strict(name, ""))?;
        std::fs::write(member.join("README.md"), "")?;
        std::fs::write(member.join("src/main.rs"), FORMATTED_MAIN_RS)?;
    }

    let out = common::run_lockpick(workspace.path())
        .env("PATH", &path)
        .output()?;
    let view = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(0_i32),
        "expected exit 0 with workspace skip list honoured, got code={code:?}, combined output=\n{view}",
        code = out.status.code(),
    );
    assert!(
        !view.contains("audit"),
        "audit should never appear (skipped via workspace metadata), combined output=\n{view}"
    );
    assert!(
        !view.contains("machete"),
        "machete should never appear (skipped via workspace metadata), combined output=\n{view}"
    );
    let report = stdout(&out);
    assert!(report.contains("OK:"), "missing success summary:\n{report}");
    Ok(())
}

#[test]
fn invalid_configuration_aborts_before_fixes() -> TestResult {
    let project = scratch_crate(
        "invalid_config",
        "[package.metadata.lockpick]\nlicense-header = \"missing\"\n[package.metadata.lockpick.coverage]\nline = 100\n",
        &[("src/main.rs", common::UNFORMATTED_MAIN_RS)],
    );
    let out = common::run_lockpick(project.path())
        .args([
            "--fix",
            "--skip",
            "check,clippy,test,doc,doc-test,machete,audit",
        ])
        .output()?;
    assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
    assert!(common::stderr(&out).contains("unknown field `line`"));
    assert!(!stdout(&out).contains("PASS"));
    assert_eq!(
        std::fs::read_to_string(project.path().join("src/main.rs"))?,
        common::UNFORMATTED_MAIN_RS
    );
    Ok(())
}

#[test]
fn metadata_failure_is_reported_before_running_checks() -> TestResult {
    let directory = tempfile::tempdir()?;
    let out = common::run_lockpick(directory.path()).output()?;
    assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
    assert!(common::stderr(&out).contains("cargo metadata failed"));
    assert!(!stdout(&out).contains("PASS"));
    Ok(())
}

#[test]
fn missing_or_empty_license_template_fails_the_gate() -> TestResult {
    let project = scratch_crate(
        "license_template",
        "[package.metadata.lockpick]\nlicense-header=\"header.txt\"\n",
        &[("src/lib.rs", "pub const VALUE: u8 = 1;\n")],
    );
    for diagnostic in ["could not read license header file", "is empty"] {
        let out = common::run_lockpick(project.path())
            .args(["--skip", "check,clippy,fmt,test,doc,doc-test,machete,audit"])
            .output()?;
        assert_eq!(out.status.code(), Some(1_i32), "{}", combined(&out));
        assert!(combined(&out).contains(diagnostic));
        std::fs::write(project.path().join("header.txt"), "")?;
    }
    Ok(())
}

#[test]
fn missing_cargo_reports_the_launch_error_before_fixes() -> TestResult {
    let project = scratch_crate("missing_cargo", "", &[("src/main.rs", "fn main() {}\n")]);
    let path = tempfile::tempdir()?;
    let out = common::run_lockpick(project.path())
        .arg("--fix")
        .env("PATH", path.path())
        .output()?;
    assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
    assert!(common::stderr(&out).contains("could not run cargo metadata:"));
    assert_eq!(
        std::fs::read_to_string(project.path().join("src/main.rs"))?,
        "fn main() {}\n"
    );
    Ok(())
}
