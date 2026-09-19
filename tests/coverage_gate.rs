// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Real coverage tools run only against an isolated fixture workspace.

mod common;
use common::process::bounded_output;

use common::{TestResult, combined, run_lockpick, scratch_crate};

#[test]
fn fresh_coverage_cannot_reuse_a_previous_passing_run() -> TestResult {
    let project = scratch_crate(
        "fresh_coverage",
        "[package.metadata.lockpick.coverage]\n",
        &[
            (
                "src/lib.rs",
                "pub fn value() -> u8 { std::hint::black_box(7) }\n",
            ),
            (
                "tests/exercise.rs",
                "#[test] fn exercise() { if std::path::Path::new(\"exercise\").exists() { assert_eq!(fresh_coverage::value(), 7); } }\n",
            ),
        ],
    );
    let marker = project.path().join("exercise");
    std::fs::write(&marker, "")?;
    let execute = || {
        bounded_output(run_lockpick(project.path()).args([
            "--skip",
            "check,clippy,fmt,doc,doc-test,machete,audit",
            "-v",
        ]))
    };
    let first = execute()?;
    assert_eq!(first.status.code(), Some(0_i32), "{}", combined(&first));
    assert!(combined(&first).contains("ok   functions"));
    std::fs::remove_file(marker)?;
    let second = execute()?;
    assert_eq!(second.status.code(), Some(1_i32), "{}", combined(&second));
    assert!(
        combined(&second).contains("FAIL functions"),
        "{}",
        combined(&second)
    );
    assert!(combined(&second).contains("(coverage)"));
    Ok(())
}

#[test]
fn locked_coverage_preserves_missing_stale_and_current_lockfiles() -> TestResult {
    let project = scratch_crate(
        "locked_coverage",
        "[package.metadata.lockpick]\nlocked=true\n[package.metadata.lockpick.coverage]\n",
        &[
            (
                "src/lib.rs",
                "pub fn value() -> u8 { std::hint::black_box(7) }\n",
            ),
            (
                "tests/exercise.rs",
                "#[test] fn exercise() { assert_eq!(locked_coverage::value(), 7); }\n",
            ),
            (
                "dependency/Cargo.toml",
                "[package]\nname=\"local_dependency\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
            ),
            ("dependency/src/lib.rs", "pub const VALUE: u8 = 1;\n"),
        ],
    );
    let lockfile = project.path().join("Cargo.lock");
    let execute = || {
        bounded_output(run_lockpick(project.path()).args([
            "--skip",
            "check,clippy,fmt,doc,doc-test,machete,audit",
            "-v",
        ]))
    };
    let missing = execute()?;
    assert_eq!(missing.status.code(), Some(1_i32), "{}", combined(&missing));
    assert!(!lockfile.exists());
    assert!(combined(&missing).contains("cargo llvm-cov clean --workspace --locked"));
    assert!(combined(&missing).contains("test       FAIL"));
    assert!(combined(&missing).contains("coverage   SKIP"));

    let generate = || {
        bounded_output(
            std::process::Command::new("cargo")
                .args(["generate-lockfile", "--offline"])
                .current_dir(project.path()),
        )
    };
    let generated = generate()?;
    assert!(generated.status.success(), "{}", combined(&generated));
    let manifest = project.path().join("Cargo.toml");
    let original = std::fs::read_to_string(&manifest)?;
    std::fs::write(
        &manifest,
        format!("{original}\n[dependencies]\nlocal_dependency={{path=\"dependency\"}}\n"),
    )?;
    let before = std::fs::read(&lockfile)?;
    let stale = execute()?;
    assert_eq!(stale.status.code(), Some(1_i32), "{}", combined(&stale));
    assert_eq!(std::fs::read(&lockfile)?, before);
    assert!(combined(&stale).contains("test       FAIL"));

    let generated = generate()?;
    assert!(generated.status.success(), "{}", combined(&generated));
    let before = std::fs::read(&lockfile)?;
    let current = execute()?;
    assert_eq!(current.status.code(), Some(0_i32), "{}", combined(&current));
    assert_eq!(std::fs::read(&lockfile)?, before);
    assert!(combined(&current).contains("ok   functions"));
    Ok(())
}
