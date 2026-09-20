// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![allow(
    unused_crate_dependencies,
    reason = "These tests run Lockpick as a subprocess; Cargo also supplies its application dependencies."
)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Real subprocess contracts for custom build directories and cancellation.

mod common;
use common::{TestResult, combined, run_lockpick, scratch_crate};

#[test]
fn running_binary_is_isolated_from_custom_cargo_output() -> TestResult {
    for use_environment in [true, false] {
        let project = scratch_crate(
            "custom_output",
            "",
            &[("src/lib.rs", "pub const VALUE: u8 = 7;\n")],
        );
        let directory = project.path().join("custom");
        std::fs::create_dir_all(directory.join("debug"))?;
        let executable = directory
            .join("debug")
            .join(format!("lockpick{}", std::env::consts::EXE_SUFFIX));
        let _copied = std::fs::copy(common::lockpick_bin(), &executable)?;
        if !use_environment {
            std::fs::create_dir_all(project.path().join(".cargo"))?;
            std::fs::write(
                project.path().join(".cargo/config.toml"),
                "[build]\ntarget-dir=\"custom\"\n",
            )?;
        }
        let environment = run_lockpick(project.path());
        let mut command = std::process::Command::new(executable);
        let _command = command.env_clear().envs(
            environment
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
        let _command = command
            .current_dir(project.path())
            .args(["--skip", "clippy,fmt,test,doc,doc-test,machete,audit"]);
        if use_environment {
            let _command = command.env("CARGO_TARGET_DIR", &directory);
        }
        let out = command.output()?;
        assert_eq!(out.status.code(), Some(0_i32), "{}", combined(&out));
        assert!(directory.join("lockpick/debug").is_dir());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn cancellation_stops_cargo_descendants() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};
    let project = common::dummy_cargo_project();
    let shim = tempfile::tempdir()?;
    let ready = shim.path().join("ready");
    let marker = shim.path().join("survived");
    let path = std::env::var_os("PATH").ok_or("PATH is missing")?;
    let cargo = std::env::split_paths(&path)
        .map(|directory| directory.join("cargo"))
        .find(|candidate| candidate.is_file())
        .ok_or("cargo is missing")?;
    let script = shim.path().join("cargo");
    std::fs::write(
        &script,
        "#!/bin/sh\nif [ \"$1\" = metadata ]; then exec \"$LOCKPICK_REAL_CARGO\" \"$@\"; fi\nsh -c 'sleep 2; echo survived > \"$LOCKPICK_MARKER\"' &\necho ready > \"$LOCKPICK_READY\"\nwait\n",
    )?;
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755))?;
    let search = std::env::join_paths(
        std::iter::once(shim.path().to_path_buf()).chain(std::env::split_paths(&path)),
    )?;
    let mut child = run_lockpick(project.path())
        .args(["--skip", "check,clippy,test,doc,doc-test,machete,audit"])
        .env("PATH", search)
        .env("LOCKPICK_REAL_CARGO", cargo)
        .env("LOCKPICK_READY", &ready)
        .env("LOCKPICK_MARKER", &marker)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let started = Instant::now();
    while !ready.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "Cargo shim did not become ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let signal = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()?;
    assert!(signal.success());
    while child.try_wait()?.is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "Lockpick did not exit after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(child.wait()?.code(), Some(143_i32));
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!marker.exists(), "a Cargo descendant survived cancellation");
    Ok(())
}

#[cfg(unix)]
#[test]
fn cancellation_stops_startup_probes_and_their_descendants() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    for probe in ["metadata", "version", "host"] {
        let project = scratch_crate(
            "startup_cancellation",
            "[package.metadata.lockpick]\nhost-target=true\n[package.metadata.lockpick.coverage]\n",
            &[("src/lib.rs", "pub const VALUE: u8 = 7;\n")],
        );
        let shim = tempfile::tempdir()?;
        let ready = shim.path().join("ready");
        let marker = shim.path().join("survived");
        let path = std::env::var_os("PATH").ok_or("PATH is missing")?;
        for name in ["cargo", "rustc"] {
            let real = std::env::split_paths(&path)
                .map(|directory| directory.join(name))
                .find(|candidate| candidate.is_file())
                .ok_or("Cargo tool is missing")?;
            let script = shim.path().join(name);
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\ncase \"$LOCKPICK_PROBE:$1:$2\" in metadata:metadata:*|version:--version:*|host:--print:host-tuple)\nsh -c 'sleep 2; echo survived > \"$LOCKPICK_MARKER\"' &\necho \"$PPID\" > \"$LOCKPICK_READY\"\nwait\nexit 1\n;;\nesac\nexec {real:?} \"$@\"\n"
                ),
            )?;
            std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755))?;
        }
        let search = std::env::join_paths(
            std::iter::once(shim.path().to_path_buf()).chain(std::env::split_paths(&path)),
        )?;
        let mut command = run_lockpick(project.path());
        let _command = command
            .args(["--skip", "check,clippy,fmt,doc,doc-test,machete,audit"])
            .env("PATH", search)
            .env("LOCKPICK_PROBE", probe)
            .env("LOCKPICK_READY", &ready)
            .env("LOCKPICK_MARKER", &marker);
        let out = std::thread::scope(|scope| -> TestResult {
            let signal = scope.spawn(|| -> std::io::Result<()> {
                let started = Instant::now();
                while !ready.exists() {
                    if started.elapsed() >= Duration::from_secs(10) {
                        return Err(std::io::Error::other("startup probe did not become ready"));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                let pid = std::fs::read_to_string(&ready)?;
                let status = std::process::Command::new("kill")
                    .args(["-TERM", pid.trim()])
                    .status()?;
                assert!(status.success());
                Ok(())
            });
            let out = common::process::bounded_output(&mut command)?;
            signal.join().expect("signal worker panicked")?;
            assert_eq!(
                out.status.code(),
                Some(143_i32),
                "{probe}: {}",
                combined(&out)
            );
            assert!(!common::stdout(&out).contains("PASS"));
            Ok(())
        });
        out?;
        std::thread::sleep(Duration::from_millis(2200));
        assert!(!marker.exists(), "{probe} descendant survived cancellation");
    }
    Ok(())
}
