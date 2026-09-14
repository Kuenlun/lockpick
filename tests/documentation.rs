// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Cargo's encoded rustdoc environment must not bypass the warning gate.

mod common;

#[test]
fn encoded_rustdoc_flags_preserve_cfg_and_deny_documentation_warnings() -> common::TestResult {
    let source = "#[cfg(not(doc_test_cfg))]\ncompile_error!(\"custom rustdoc cfg was lost\");\n/// See [MissingType].\npub struct Documented;\n";
    let project = common::scratch_crate("rustdoc_flags", "", &[("src/lib.rs", source)]);
    let out = common::run_lockpick(project.path())
        .args(["--skip", "check,clippy,fmt,test,doc-test,machete,audit"])
        .env("RUSTDOCFLAGS", "--invalid-plain-flag")
        .env(
            "CARGO_ENCODED_RUSTDOCFLAGS",
            "--cfg\u{1f}doc_test_cfg\u{1f}--check-cfg\u{1f}cfg(doc_test_cfg)",
        )
        .output()?;
    let view = common::combined(&out);
    assert_eq!(out.status.code(), Some(1), "{view}");
    assert!(view.contains("MissingType"), "{view}");
    assert!(!view.contains("custom rustdoc cfg was lost"), "{view}");
    assert!(!view.contains("Unrecognized option"), "{view}");
    Ok(())
}
