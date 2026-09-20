// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Documentation checks preserve Cargo lint levels and rustdoc flag precedence.

mod common;

#[test]
fn rustdoc_preserves_project_levels_and_user_flags() -> common::TestResult {
    let source = "#[cfg(not(doc_test_cfg))]\ncompile_error!(\"custom rustdoc cfg was lost\");\n/// See [MissingType].\npub struct Documented;\n";
    for level in ["allow", "warn", "deny"] {
        let config = format!("[lints.rustdoc]\nbroken_intra_doc_links={level:?}\n");
        let project = common::scratch_crate("rustdoc_flags", &config, &[("src/lib.rs", source)]);
        for encoded in [false, true] {
            let mut command = common::run_lockpick(project.path());
            let _command = command.args(["--skip", "check,clippy,fmt,test,doc-test,machete,audit"]);
            if encoded {
                let _command = command.env("RUSTDOCFLAGS", "--invalid-plain-flag").env(
                    "CARGO_ENCODED_RUSTDOCFLAGS",
                    "--cfg\u{1f}doc_test_cfg\u{1f}--check-cfg\u{1f}cfg(doc_test_cfg)",
                );
            } else {
                let _command = command.env(
                    "RUSTDOCFLAGS",
                    "--cfg doc_test_cfg --check-cfg cfg(doc_test_cfg)",
                );
            }
            let out = command.output()?;
            let view = common::combined(&out);
            assert_eq!(out.status.success(), level != "deny", "{level}: {view}");
            assert!(!view.contains("custom rustdoc cfg was lost"), "{view}");
            assert!(!view.contains("Unrecognized option"), "{view}");
        }
    }
    Ok(())
}
