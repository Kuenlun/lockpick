// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![allow(
    unused_crate_dependencies,
    reason = "These tests run Lockpick as a subprocess; Cargo also supplies its application dependencies."
)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Manifest lint levels govern normal checks, target checks, and automatic fixes.

mod common;

const SOURCE: &str = "pub fn answer() -> u8 {\n    42\n}\n";
const SKIP_OTHER_CHECKS: &str = "check,fmt,test,doc,doc-test,machete,audit";

#[test]
fn clippy_preserves_individual_and_group_levels() -> common::TestResult {
    for level in ["allow", "warn", "deny"] {
        let config = format!(
            "[lints.clippy]\npedantic={{level=\"deny\",priority=-1}}\nnursery={{level=\"allow\",priority=-1}}\ncargo={{level=\"allow\",priority=-1}}\nmust_use_candidate={level:?}\n[[package.metadata.lockpick.target-checks]]\nartifacts=\"lib\"\nprofile=\"release\"\n"
        );
        let project = common::scratch_crate("lint_policy", &config, &[("src/lib.rs", SOURCE)]);
        let out = common::run_lockpick(project.path())
            .args(["--skip", SKIP_OTHER_CHECKS, "-v"])
            .output()?;
        let view = common::combined(&out);
        assert_eq!(out.status.success(), level != "deny", "{level}: {view}");
        assert!(view.contains("--profile release"), "{view}");
        if level == "deny" {
            assert!(view.contains("must_use_candidate"), "{view}");
        }
    }
    Ok(())
}

#[test]
fn clippy_fix_leaves_explicitly_allowed_code_unchanged() -> common::TestResult {
    let config = "[lints.clippy]\npedantic=\"allow\"\nnursery=\"allow\"\ncargo=\"allow\"\n";
    let project = common::scratch_crate("lint_fix", config, &[("src/lib.rs", SOURCE)]);
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(project.path())
        .output()?;
    assert!(init.status.success(), "{}", common::combined(&init));
    let out = common::run_lockpick(project.path())
        .args(["--fix", "--skip", SKIP_OTHER_CHECKS])
        .output()?;
    assert!(out.status.success(), "{}", common::combined(&out));
    assert_eq!(
        std::fs::read_to_string(project.path().join("src/lib.rs"))?,
        SOURCE
    );
    Ok(())
}

#[test]
fn workspace_members_keep_their_own_lint_policies() -> common::TestResult {
    let project = tempfile::tempdir()?;
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[workspace]\nmembers=[\"inherited\",\"local\"]\nresolver=\"3\"\n[workspace.lints.clippy]\npedantic=\"allow\"\nnursery=\"allow\"\ncargo=\"allow\"\n",
    )?;
    for name in ["inherited", "local"] {
        std::fs::create_dir_all(project.path().join(name).join("src"))?;
        let policy = if name == "inherited" {
            "[lints]\nworkspace=true\n"
        } else {
            "[lints.clippy]\nmust_use_candidate=\"deny\"\n"
        };
        std::fs::write(
            project.path().join(name).join("Cargo.toml"),
            common::cargo_toml_strict(name, policy),
        )?;
        std::fs::write(project.path().join(name).join("src/lib.rs"), SOURCE)?;
    }
    let out = common::run_lockpick(project.path())
        .args(["--skip", SKIP_OTHER_CHECKS])
        .output()?;
    let view = common::combined(&out);
    assert!(!out.status.success(), "{view}");
    let view = view.replace('\\', "/");
    assert!(view.contains("local/src/lib.rs"), "{view}");
    assert!(!view.contains("inherited/src/lib.rs"), "{view}");
    Ok(())
}
