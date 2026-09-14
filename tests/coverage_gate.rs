// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Real coverage tools run only against an isolated fixture workspace.

mod common;

use common::{TestResult, bounded_output, combined, run_lockpick, scratch_crate};

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
    assert_eq!(first.status.code(), Some(0), "{}", combined(&first));
    assert!(combined(&first).contains("ok   functions"));
    std::fs::remove_file(marker)?;
    let second = execute()?;
    assert_eq!(second.status.code(), Some(1), "{}", combined(&second));
    assert!(
        combined(&second).contains("FAIL functions"),
        "{}",
        combined(&second)
    );
    assert!(combined(&second).contains("(coverage)"));
    Ok(())
}
