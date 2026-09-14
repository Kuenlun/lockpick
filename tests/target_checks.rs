// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Real Cargo checks for host execution, embedded compilation and lockfile policy.

mod common;
use common::{TestResult, combined, run_lockpick, scratch_crate};

const EMBEDDED: &str = "thumbv8m.main-none-eabihf";

#[test]
fn host_tests_and_embedded_profiles_share_one_gate() -> TestResult {
    let config = format!(
        "[package.metadata.lockpick]\nlocked=true\nhost-target=true\n[[package.metadata.lockpick.target-checks]]\ntarget={EMBEDDED:?}\nartifacts=\"lib\"\n[[package.metadata.lockpick.target-checks]]\ntarget={EMBEDDED:?}\nartifacts=\"lib\"\nprofile=\"release\"\n"
    );
    let project = scratch_crate(
        "embedded_profiles",
        &config,
        &[
            (
                "src/lib.rs",
                "#![no_std]\n#[must_use]\npub const fn identity(value: u8) -> u8 {\n    value\n}\n",
            ),
            (
                "tests/host.rs",
                "#[test]\nfn executes_on_host() {\n    assert_eq!(embedded_profiles::identity(7), 7);\n}\n",
            ),
            (
                ".cargo/config.toml",
                "[build]\ntarget=\"thumbv8m.main-none-eabihf\"\n",
            ),
        ],
    );
    let lock = std::process::Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(project.path())
        .output()?;
    assert!(lock.status.success(), "{}", combined(&lock));
    let out = run_lockpick(project.path())
        .args(["--skip", "machete,audit", "-v"])
        .output()?;
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let view = combined(&out);
    assert!(view.contains("executes_on_host"));
    assert!(view.contains("--target host-tuple"));
    assert!(view.contains("--profile release --locked --target thumbv8m.main-none-eabihf"));
    assert!(view.contains("targets"));
    Ok(())
}

#[test]
fn locked_policy_rejects_a_missing_lockfile_without_creating_one() -> TestResult {
    let project = scratch_crate(
        "locked_fixture",
        "[package.metadata.lockpick]\nlocked=true\n",
        &[("src/lib.rs", "pub const VALUE: u8 = 7;\n")],
    );
    let out = run_lockpick(project.path())
        .args(["--skip", "clippy,fmt,test,doc,doc-test,machete,audit"])
        .output()?;
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    assert!(!project.path().join("Cargo.lock").exists());
    assert!(combined(&out).contains("--locked"));
    Ok(())
}

#[test]
fn invalid_feature_policy_is_rejected_before_any_checks() -> TestResult {
    for field in [
        "no-default-features=true",
        "features=[\"motor\"]",
        "target=\"--release\"",
        "profile=\"\"",
    ] {
        let config = format!("[[package.metadata.lockpick.target-checks]]\n{field}\n");
        let project = scratch_crate(
            "invalid_target",
            &config,
            &[("src/lib.rs", "pub const VALUE: u8 = 7;\n")],
        );
        let out = run_lockpick(project.path()).output()?;
        assert_eq!(out.status.code(), Some(2), "{}", combined(&out));
        assert!(combined(&out).contains("target-checks"));
    }
    Ok(())
}

#[test]
fn embedded_binaries_use_selected_features_and_enforce_release_profile() -> TestResult {
    let config = format!(
        "[features]\ndefault=[\"bad\"]\nbad=[]\ndevice=[]\n[[package.metadata.lockpick.target-checks]]\ntarget={EMBEDDED:?}\nartifacts=\"bins\"\npackages=[\"embedded_binary\"]\nall-features=false\nno-default-features=true\nfeatures=[\"device\"]\nprofile=\"release\"\n"
    );
    let source = "#![no_std]\n#![no_main]\n#[cfg(any(feature = \"bad\", not(feature = \"device\")))]\ncompile_error!(\"incorrect feature selection\");\n#[panic_handler]\nfn panic(_: &core::panic::PanicInfo<'_>) -> ! {\n    loop {\n        core::hint::spin_loop();\n    }\n}\n";
    let project = scratch_crate("embedded_binary", &config, &[("src/main.rs", source)]);
    let execute = || {
        run_lockpick(project.path())
            .args(["--skip", "check,clippy,fmt,test,doc,doc-test,machete,audit"])
            .output()
    };
    let out = execute()?;
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    std::fs::write(
        project.path().join("src/main.rs"),
        format!(
            "{source}\n#[cfg(not(debug_assertions))]\ncompile_error!(\"release check reached\");\n"
        ),
    )?;
    let out = execute()?;
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    assert!(combined(&out).contains("release check reached"));
    Ok(())
}
