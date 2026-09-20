// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![allow(
    unused_crate_dependencies,
    reason = "These tests run Lockpick as a subprocess; Cargo also supplies its application dependencies."
)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

//! Deadline cleanup must reach descendants outside the immediate child's process group.

mod common;

use std::process::Command;
use std::time::Duration;

#[test]
fn timeout_terminates_descendants_in_separate_process_groups() -> common::TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("fixture.rs");
    let executable = directory
        .path()
        .join(format!("fixture{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(
        &source,
        r#"
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let depth: u8 = std::env::args().nth(1).ok_or("missing depth")?.parse()?;
    if depth == 0 {
        std::fs::write("ready", "")?;
        std::thread::sleep(std::time::Duration::from_secs(3));
        std::fs::write("survived", "")?;
    } else {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.arg((depth - 1).to_string());
        #[cfg(unix)] {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.spawn()?.wait()?;
    }
    Ok(())
}
"#,
    )?;
    let build = common::process::bounded_output(
        Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable),
    )?;
    assert!(build.status.success(), "{}", common::combined(&build));
    #[cfg(not(unix))]
    let mut command = Command::new(&executable);
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("sh");
        let _command = command
            .args(["-c", "trap '' TERM; exec \"$@\"", "fixture"])
            .arg(&executable);
        command
    };
    let error = common::process::bounded_output_with_timeout(
        command.arg("2").current_dir(directory.path()),
        Duration::from_secs(1),
    )
    .err()
    .ok_or("fixture did not time out")?;
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(directory.path().join("ready").exists());
    std::thread::sleep(Duration::from_millis(3200));
    assert!(
        !directory.path().join("survived").exists(),
        "a descendant survived the fixture deadline"
    );
    Ok(())
}
