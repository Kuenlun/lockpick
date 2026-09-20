// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![allow(
    unused_crate_dependencies,
    reason = "These tests run Lockpick as a subprocess; Cargo also supplies its application dependencies."
)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Pipeline contracts against bounded, deterministic external tools on every platform.

mod common;
use common::process::bounded_output;

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use common::{TestResult, combined, run_lockpick, stderr, stdout};
use serde_json::json;

fn tool_binary() -> Result<&'static Path, Box<dyn std::error::Error>> {
    static BINARY: OnceLock<std::io::Result<tempfile::TempDir>> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let directory = tempfile::tempdir()?;
            let source = directory.path().join("tool.rs");
            std::fs::write(&source, include_str!("fixtures/cargo.rs"))?;
            let output = bounded_output(
                Command::new("rustc")
                    .arg(&source)
                    .args(["--edition", "2024", "-D", "warnings", "-o"])
                    .arg(
                        directory
                            .path()
                            .join(format!("tool{}", std::env::consts::EXE_SUFFIX)),
                    ),
            )
            .map_err(std::io::Error::other)?;
            if !output.status.success() {
                return Err(std::io::Error::other(combined(&output)));
            }
            Ok(directory)
        })
        .as_ref()
        .map(tempfile::TempDir::path)
        .map_err(|error| error.to_string().into())
}

struct Fixture {
    directory: tempfile::TempDir,
    metadata: serde_json::Value,
}

impl Fixture {
    fn new(config: &serde_json::Value) -> Result<Self, Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        for name in [
            "cargo",
            "rustc",
            "cargo-llvm-cov",
            "cargo-machete",
            "cargo-audit",
        ] {
            let _bytes = std::fs::copy(
                tool_binary()?.join(format!("tool{}", std::env::consts::EXE_SUFFIX)),
                root.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
            )?;
        }
        let metadata = json!({
            "workspace_root": root,
            "target_directory": root.join("target"),
            "packages": [{"manifest_path": root.join("Cargo.toml"), "targets": [{"kind": ["lib"]}]}],
            "metadata": {"lockpick": config}
        });
        Ok(Self {
            directory,
            metadata,
        })
    }

    fn command(&self) -> Command {
        let mut command = run_lockpick(self.directory.path());
        let _command = command
            .env("PATH", self.directory.path())
            .env(
                "RUSTC",
                self.directory
                    .path()
                    .join(format!("rustc{}", std::env::consts::EXE_SUFFIX)),
            )
            .env("LOCKPICK_TEST_LOG", self.directory.path().join("commands"))
            .env("LOCKPICK_TEST_METADATA", self.metadata.to_string())
            .env("LOCKPICK_TEST_CHANNEL", "nightly")
            .env("LOCKPICK_TEST_HOST", "test-host")
            .env(
                "LOCKPICK_TEST_REPORT",
                json!({
                    "type": "llvm.coverage.json.export", "version": "2.0.1",
                    "data": [{"files": [{"filename": "src/lib.rs"}], "totals": {
                        "functions": {"count": 1_i32, "covered": 1_i32},
                        "lines": {"count": 1_i32, "covered": 1_i32},
                        "regions": {"count": 1_i32, "covered": 1_i32},
                        "branches": {"count": 2_i32, "covered": 2_i32}
                    }}]
                })
                .to_string(),
            );
        command
    }

    fn log(&self) -> std::io::Result<String> {
        std::fs::read_to_string(self.directory.path().join("commands"))
    }
}

#[test]
fn external_checks_preserve_diagnostics_policy_and_exit_status() -> TestResult {
    for failure in ["", "audit", "machete"] {
        let fixture = Fixture::new(&json!({}))?;
        let out = bounded_output(
            fixture
                .command()
                .args(["-v", "--skip", "check,clippy,test,doc,doc-test,fmt"])
                .env("LOCKPICK_TEST_FAIL", failure),
        )?;
        assert_eq!(
            out.status.success(),
            failure.is_empty(),
            "{}",
            combined(&out)
        );
        let log = fixture.log()?;
        assert!(
            log.contains("[\"audit\", \"--deny\", \"warnings\"]"),
            "{log}"
        );
        assert!(log.contains("[\"machete\"]"), "{log}");
        for sub in ["audit", "machete"] {
            assert!(stdout(&out).contains(&format!("{sub} stdout")));
            assert!(stdout(&out).contains(&format!("{sub} stderr")));
            assert!(stderr(&out).contains(&format!("cargo {sub}")));
        }
        if !failure.is_empty() {
            assert!(stdout(&out).contains(&format!("Failed: 1/2 ({failure})")));
        }
    }
    Ok(())
}

#[test]
fn malformed_metadata_stops_before_checks_and_fixes() -> TestResult {
    let fixture = Fixture::new(&json!({}))?;
    let out = bounded_output(
        fixture
            .command()
            .arg("--fix")
            .env("LOCKPICK_TEST_METADATA", "not JSON"),
    )?;
    assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
    assert!(stderr(&out).contains("invalid cargo metadata JSON"));
    assert_eq!(fixture.log()?.lines().count(), 1);
    Ok(())
}

#[test]
fn skip_notes_reflect_explicitly_disabled_configured_gates() -> TestResult {
    let fixture = Fixture::new(&json!({"coverage": {}, "license-header": "missing-header"}))?;
    let out = bounded_output(fixture.command().args([
        "--skip",
        "test,coverage,license,check,clippy,doc,doc-test,machete,audit",
    ]))?;
    assert!(out.status.success(), "{}", combined(&out));
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
    let log = fixture.log()?;
    assert!(log.contains("[\"fmt\""));
    assert!(!log.contains("[\"llvm-cov\""));
    Ok(())
}

#[test]
fn cargo_children_drop_package_context_and_keep_explicit_color() -> TestResult {
    let fixture = Fixture::new(&json!({}))?;
    let out = bounded_output(
        fixture
            .command()
            .args([
                "--color",
                "always",
                "--skip",
                "check,clippy,test,doc,doc-test,machete,audit",
            ])
            .env("CARGO_PKG_NAME", "parent-package")
            .env("CARGO_BIN_NAME", "parent-binary")
            .env("CARGO_CRATE_NAME", "parent-crate")
            .env("CARGO_MANIFEST_DIR", "parent-manifest")
            .env("CUSTOM_ENV", "preserved")
            .env("NO_COLOR", "1"),
    )?;
    assert!(out.status.success(), "{}", combined(&out));
    let log = fixture.log()?;
    assert!(!log.contains("parent-"), "{log}");
    assert!(log.contains("\"CUSTOM_ENV\", \"preserved\""), "{log}");
    assert!(log.contains("\"CARGO_TERM_COLOR\", \"always\""), "{log}");
    assert!(log.contains("\"--color\", \"always\""), "{log}");
    assert!(stdout(&out).contains('\u{1b}'));
    Ok(())
}

#[test]
fn stable_coverage_omits_branches_and_explicit_threshold_requires_nightly() -> TestResult {
    for branches in [None, Some(100_i32)] {
        let config = branches.map_or_else(
            || json!({"coverage": {}}),
            |threshold| json!({"coverage": {"branches": threshold}}),
        );
        let fixture = Fixture::new(&config)?;
        let out = bounded_output(
            fixture
                .command()
                .args([
                    "-v",
                    "--skip",
                    "check,clippy,fmt,doc,doc-test,machete,audit",
                ])
                .env("LOCKPICK_TEST_CHANNEL", "stable"),
        )?;
        if branches.is_some() {
            assert_eq!(out.status.code(), Some(4_i32), "{}", combined(&out));
            assert!(stderr(&out).contains("coverage.branches"));
            assert_eq!(fixture.log()?.lines().count(), 1);
        } else {
            assert!(out.status.success(), "{}", combined(&out));
            assert!(stderr(&out).contains("branch coverage disabled: requires nightly"));
            assert!(!fixture.log()?.contains("--branch"));
            assert!(!stdout(&out).contains("ok   branches"));
        }
    }
    Ok(())
}

#[test]
fn invalid_compiler_host_is_rejected_before_fixing() -> TestResult {
    for (host, failure) in [
        ("", ""),
        ("-option", ""),
        ("invalid target", ""),
        ("valid-host", "1"),
    ] {
        let fixture = Fixture::new(&json!({"coverage": {}, "host-target": true}))?;
        let out = bounded_output(
            fixture
                .command()
                .arg("--fix")
                .env("LOCKPICK_TEST_HOST", host)
                .env("LOCKPICK_TEST_FAIL_HOST", failure),
        )?;
        assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
        assert!(stderr(&out).contains("could not determine the coverage host target"));
        assert_eq!(fixture.log()?.lines().count(), 1);
    }
    let fixture = Fixture::new(&json!({"coverage": {}, "host-target": true}))?;
    std::fs::remove_file(
        fixture
            .directory
            .path()
            .join(format!("rustc{}", std::env::consts::EXE_SUFFIX)),
    )?;
    let out = bounded_output(fixture.command().arg("--fix"))?;
    assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
    assert!(stderr(&out).contains("could not query the coverage host target"));
    assert_eq!(fixture.log()?.lines().count(), 1);
    Ok(())
}

#[test]
fn failed_fix_launch_reports_the_os_error_and_stops_the_pipeline() -> TestResult {
    let fixture = Fixture::new(&json!({}))?;
    let out = bounded_output(
        fixture
            .command()
            .arg("--fix")
            .env("LOCKPICK_TEST_REMOVE_CARGO", "1"),
    )?;
    assert_eq!(out.status.code(), Some(1_i32), "{}", combined(&out));
    assert!(stderr(&out).contains("fix: failed to launch cargo clippy:"));
    assert_eq!(fixture.log()?.lines().count(), 1);
    assert!(!stdout(&out).contains("PASS"));
    Ok(())
}

#[test]
fn workspace_policy_is_defaulted_or_rejected_without_choosing_a_member() -> TestResult {
    for member_policy in [None, Some(json!({"skip": ["fmt"]}))] {
        let mut fixture = Fixture::new(&json!({}))?;
        *fixture.metadata.get_mut("metadata").unwrap() = json!(null);
        *fixture.metadata.get_mut("packages").unwrap() = json!([
            {"metadata": {"lockpick": member_policy}}, {"metadata": {}}
        ]);
        if member_policy.is_none() {
            *fixture.metadata.get_mut("packages").unwrap() = json!([{}, {}]);
        }
        let out = bounded_output(
            fixture
                .command()
                .args(["--skip", "check,clippy,test,doc,doc-test,machete,audit"]),
        )?;
        if member_policy.is_some() {
            assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
            assert!(stderr(&out).contains("multi-crate workspace"));
            assert_eq!(fixture.log()?.lines().count(), 1);
        } else {
            assert!(out.status.success(), "{}", combined(&out));
            assert!(stdout(&out).contains("fmt        PASS"));
        }
    }
    Ok(())
}

#[test]
fn malformed_policy_values_stop_before_any_tool_runs() -> TestResult {
    for config in [
        json!({"skip": [42_i32]}),
        json!({"target-checks": [{"profile": "bad\nprofile"}]}),
    ] {
        let fixture = Fixture::new(&config)?;
        let out = bounded_output(fixture.command().arg("--fix"))?;
        assert_eq!(out.status.code(), Some(2_i32), "{}", combined(&out));
        assert!(stderr(&out).contains("invalid Lockpick configuration"));
        assert_eq!(fixture.log()?.lines().count(), 1);
    }
    Ok(())
}

#[test]
fn license_selection_excludes_its_template_and_rejects_an_empty_scan() -> TestResult {
    for selection in [json!(["*.rs"]), json!([])] {
        let fixture = Fixture::new(
            &json!({"license-header": "header.rs", "license-header-globs": selection}),
        )?;
        std::fs::write(fixture.directory.path().join("header.rs"), "// license\n")?;
        std::fs::write(fixture.directory.path().join("source.rs"), "// license\n")?;
        let out = bounded_output(fixture.command().args([
            "-v",
            "--skip",
            "check,clippy,fmt,test,doc,doc-test,machete,audit",
        ]))?;
        if selection == json!([]) {
            assert_eq!(out.status.code(), Some(1_i32), "{}", combined(&out));
            assert!(stdout(&out).contains("license-header-globs matched no source files"));
        } else {
            assert!(out.status.success(), "{}", combined(&out));
            assert!(stdout(&out).contains("1 file(s) checked"));
        }
        assert_eq!(fixture.log()?.lines().count(), 1);
    }
    Ok(())
}

#[test]
fn inaccessible_directories_and_dangling_sources_fail_the_license_gate() -> TestResult {
    let fixture = Fixture::new(
        &json!({"license-header": "header.txt", "license-header-globs": ["*/*", "*.rs"]}),
    )?;
    let root = fixture.directory.path();
    std::fs::write(root.join("header.txt"), "// license\n")?;
    let private = root.join("private");
    std::fs::create_dir(&private)?;
    std::fs::write(private.join("source.rs"), "// license\n")?;
    let execute = || {
        bounded_output(
            fixture
                .command()
                .args(["--skip", "check,clippy,fmt,test,doc,doc-test,machete,audit"]),
        )
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o000))?;
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME")?;
        let deny = bounded_output(
            Command::new("icacls")
                .arg(&private)
                .args(["/deny", &format!("{user}:(RD)")]),
        )?;
        assert!(deny.status.success(), "{}", combined(&deny));
    }
    let inaccessible = execute();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME")?;
        let restore = bounded_output(
            Command::new("icacls")
                .arg(&private)
                .args(["/remove:d", &user]),
        )?;
        assert!(restore.status.success(), "{}", combined(&restore));
    }
    let inaccessible = inaccessible?;
    assert_eq!(
        inaccessible.status.code(),
        Some(1_i32),
        "{}",
        combined(&inaccessible)
    );
    assert!(
        stdout(&inaccessible).contains("could not scan license sources"),
        "{}",
        combined(&inaccessible)
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink("absent", root.join("source.rs"))?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_file("absent", root.join("source.rs"))?;
    let dangling = execute()?;
    assert_eq!(
        dangling.status.code(),
        Some(1_i32),
        "{}",
        combined(&dangling)
    );
    assert!(stdout(&dangling).contains("could not inspect license source"));
    assert!(stdout(&dangling).contains("source.rs"));
    Ok(())
}

#[test]
fn missing_tools_are_reported_together_and_coverage_remains_opt_in() -> TestResult {
    for coverage in [false, true] {
        let config = if coverage {
            json!({"coverage": {}})
        } else {
            json!({})
        };
        let fixture = Fixture::new(&config)?;
        for tool in ["cargo-llvm-cov", "cargo-machete", "cargo-audit"] {
            std::fs::remove_file(
                fixture
                    .directory
                    .path()
                    .join(format!("{tool}{}", std::env::consts::EXE_SUFFIX)),
            )?;
        }
        let out = bounded_output(&mut fixture.command())?;
        assert_eq!(out.status.code(), Some(3_i32), "{}", combined(&out));
        let diagnostic = stderr(&out);
        assert_eq!(
            diagnostic.contains("cargo-llvm-cov"),
            coverage,
            "{diagnostic}"
        );
        for (binary, skip) in [("cargo-machete", "machete"), ("cargo-audit", "audit")] {
            assert!(diagnostic.contains(binary), "{diagnostic}");
            assert!(
                diagnostic.contains(&format!("--skip {skip}")),
                "{diagnostic}"
            );
        }
        let install = if coverage {
            "cargo install cargo-llvm-cov cargo-machete cargo-audit"
        } else {
            "cargo install cargo-machete cargo-audit"
        };
        assert!(diagnostic.contains(install), "{diagnostic}");
        assert_eq!(fixture.log()?.lines().count(), 1);
    }
    Ok(())
}
