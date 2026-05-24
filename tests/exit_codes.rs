// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]
#![allow(clippy::unwrap_used)]

//! End-to-end coverage of every non-success branch in `dispatch`:
//! `1` (a check failed), `3` (missing tool) and `4`
//! (coverage.branches on stable). Exit `0` and `2` live in the other
//! suites.

mod common;

use common::{BROKEN_MAIN_RS, TestResult, run_lockpick, scratch_crate, stdout};
#[cfg(unix)]
use common::{FORMATTED_MAIN_RS, dummy_cargo_project, stderr};

#[test]
fn failing_check_returns_one_and_lists_label() -> TestResult {
    let project = scratch_crate("broken", "", &[("src/main.rs", BROKEN_MAIN_RS)]);
    let out = run_lockpick(project.path())
        .args([
            "--skip", "coverage", "--skip", "machete", "--skip", "audit", "--skip", "license",
        ])
        .output()?;
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 on check failure, got code={code:?} stdout=\n{out_text}",
        code = out.status.code(),
        out_text = stdout(&out),
    );
    let report = stdout(&out);
    assert!(
        report.contains("Failed: 1/"),
        "missing summary line:\n{report}"
    );
    assert!(
        report.contains("(check)"),
        "summary should name the failing label `check`:\n{report}"
    );
    assert!(report.contains("FAIL"), "missing FAIL marker:\n{report}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn missing_tool_returns_exit_three_with_install_hint() -> TestResult {
    // PATH is reduced to a tempdir holding only `cargo` and `rustc`,
    // so cargo metadata still works but every optional plugin reads as
    // absent. The hint must enumerate all three binaries, combine them
    // into a single `cargo install` line, and offer a `--skip` for
    // each (order-independent so a future re-shuffle of
    // `require_tooling` does not silently break the test).
    let (_path_dir, path) = common::sanitized_path()?;
    let project = dummy_cargo_project();

    let out = run_lockpick(project.path()).env("PATH", &path).output()?;
    assert_eq!(
        out.status.code(),
        Some(3),
        "expected exit 3 on missing tools, got code={code:?} stderr=\n{err}",
        code = out.status.code(),
        err = stderr(&out),
    );
    let err = stderr(&out);
    for binary in ["cargo-llvm-cov", "cargo-machete", "cargo-audit"] {
        assert!(err.contains(binary), "missing `{binary}` in stderr:\n{err}");
    }
    assert!(
        err.contains("cargo install cargo-llvm-cov cargo-machete cargo-audit"),
        "missing combined install hint:\n{err}"
    );
    for skip in ["--skip coverage", "--skip machete", "--skip audit"] {
        assert!(
            err.contains(skip),
            "missing escape hatch `{skip}` in stderr:\n{err}"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn branches_on_stable_exits_four() -> TestResult {
    // Shim a `rustc` that lies on `--version` (claiming stable) while
    // execing the real binary for everything else, so cargo's own
    // toolchain probes keep working. Symlink the real cargo and the
    // three optional plugins next to it so the run reaches the
    // branches gate instead of tripping the missing-tool arm first.
    use std::os::unix::fs::PermissionsExt;

    let shim_dir = tempfile::tempdir()?;
    let bin = shim_dir.path().join("bin");
    std::fs::create_dir_all(&bin)?;

    for name in ["cargo", "cargo-llvm-cov", "cargo-machete", "cargo-audit"] {
        let src = resolve_on_path(name)?;
        std::os::unix::fs::symlink(&src, bin.join(name))?;
    }
    let real_rustc = resolve_on_path("rustc")?;

    // Write the shim as a regular file, NOT a symlink. Writing through
    // a symlink would clobber the real rustc on disk.
    let rustc_shim = bin.join("rustc");
    let script = "\
#!/bin/sh\n\
if [ \"$1\" = \"--version\" ]; then\n\
    echo \"rustc 1.85.0 (4d91de4e4 2025-02-17)\"\n\
    exit 0\n\
fi\n\
exec \"$LOCKPICK_TEST_REAL_RUSTC\" \"$@\"\n";
    std::fs::write(&rustc_shim, script)?;
    let mut perms = std::fs::metadata(&rustc_shim)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&rustc_shim, perms)?;

    let project = scratch_crate(
        "branches_stable",
        "[package.metadata.lockpick.coverage]\nbranches = 80\n",
        &[("src/main.rs", FORMATTED_MAIN_RS)],
    );

    let out = run_lockpick(project.path())
        .env("PATH", bin.to_str().ok_or("non-UTF8 shim dir")?)
        .env("RUSTC", &real_rustc)
        .env("LOCKPICK_TEST_REAL_RUSTC", &real_rustc)
        .output()?;
    assert_eq!(
        out.status.code(),
        Some(4),
        "expected exit 4 on coverage.branches+stable, got code={code:?} stderr=\n{err}",
        code = out.status.code(),
        err = stderr(&out),
    );
    let err = stderr(&out);
    assert!(
        err.contains("coverage.branches"),
        "missing offending key in stderr:\n{err}"
    );
    assert!(
        err.contains("nightly"),
        "missing nightly requirement in stderr:\n{err}"
    );
    assert!(
        err.contains("rustup toolchain install nightly"),
        "missing install hint in stderr:\n{err}"
    );
    Ok(())
}

/// Locate `name` on the harness PATH, returning the absolute path.
/// Replicated locally because integration test binaries cannot share
/// `#[cfg(unix)]`-gated helpers with `mod common` without leaking the
/// `cfg` to other files.
#[cfg(unix)]
fn resolve_on_path(name: &str) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::var_os("PATH").ok_or("PATH unset")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("could not resolve `{name}` on PATH").into())
}
