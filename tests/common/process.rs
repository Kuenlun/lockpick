// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![cfg_attr(coverage_nightly, coverage(off))]

use std::process::{Command, Output};

/// Capture a real-tool run without an unbounded wait or full output pipes.
pub(crate) fn bounded_output(command: &mut Command) -> std::io::Result<Output> {
    bounded_output_with_timeout(command, std::time::Duration::from_secs(60))
}

/// Keep timeout regression tests short without reducing real Cargo build deadlines.
pub(crate) fn bounded_output_with_timeout(
    command: &mut Command,
    timeout: std::time::Duration,
) -> std::io::Result<Output> {
    use std::io::{Read, Seek};
    use std::time::{Duration, Instant};
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let mut child = command
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?)
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            terminate_fixture(&mut child)?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Lockpick fixture exceeded its deadline",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    stdout.rewind()?;
    stderr.rewind()?;
    let mut out = Output {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let _stdout_len = stdout.read_to_end(&mut out.stdout)?;
    let _stderr_len = stderr.read_to_end(&mut out.stderr)?;
    Ok(out)
}

/// Cargo creates separate process groups, so killing only Lockpick's group is insufficient.
#[cfg(unix)]
fn terminate_fixture(child: &mut std::process::Child) -> std::io::Result<()> {
    let mut stopped = Vec::new();
    let discovery = stop_descendants(child.id(), &mut stopped);
    // Always release stopped processes, including when discovery failed partway through.
    let mut cleanup = Ok(());
    for pid in stopped.into_iter().rev() {
        if let Err(error) = signal_process("-KILL", pid) {
            cleanup = Err(error);
        }
    }
    let _kill_result = child.kill();
    let reaped = child.wait();
    discovery?;
    cleanup?;
    reaped.map(|_| ())
}

#[cfg(unix)]
fn stop_descendants(root: u32, stopped: &mut Vec<u32>) -> std::io::Result<()> {
    let mut pending = vec![root];
    while let Some(pid) = pending.pop() {
        if !signal_process("-STOP", pid)? {
            continue;
        }
        stopped.push(pid);
        // Stop each parent before looking up its children, preventing new forks during traversal.
        let snapshot = Command::new("ps")
            .args(["-A", "-o", "pid=", "-o", "ppid="])
            .output()?;
        if !snapshot.status.success() {
            return Err(std::io::Error::other(
                "could not enumerate fixture descendants",
            ));
        }
        for line in String::from_utf8_lossy(&snapshot.stdout).lines() {
            let mut fields = line.split_whitespace();
            let child = fields.next().and_then(|field| field.parse::<u32>().ok());
            let parent = fields.next().and_then(|field| field.parse::<u32>().ok());
            if parent == Some(pid)
                && let Some(child) = child
            {
                pending.push(child);
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn signal_process(signal: &str, pid: u32) -> std::io::Result<bool> {
    Command::new("kill")
        .args([signal, "--", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
}

#[cfg(windows)]
fn terminate_fixture(child: &mut std::process::Child) -> std::io::Result<()> {
    let killed = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .output();
    let _kill_result = child.kill();
    let reaped = child.wait();
    if !killed?.status.success() {
        return Err(std::io::Error::other(
            "could not terminate fixture descendants",
        ));
    }
    reaped.map(|_| ())
}
