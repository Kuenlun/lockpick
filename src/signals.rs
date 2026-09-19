// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! SIGINT/SIGTERM capture and child-process signal forwarding.
//!
//! Without this, a `kill -INT` mid-pipeline would let lockpick mistake
//! the cargo children's signal exits for ordinary check failures and
//! return `1` instead of the canonical `128 + signum`. Forwarding also
//! handles the explicit `kill -INT $lockpick_pid` case, where the
//! terminal does not broadcast to children.

use std::collections::HashSet;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// Process-wide registry of in-flight cargo child PIDs plus the latest
/// captured signal number.
pub(crate) struct State {
    received: AtomicI32,
    children: Mutex<HashSet<u32>>,
}

impl State {
    fn new() -> Self {
        Self {
            received: AtomicI32::new(0),
            children: Mutex::new(HashSet::new()),
        }
    }

    /// Signal that interrupted the run, or `None` if it ran to
    /// completion. First signal wins so a follow-up SIGTERM cannot
    /// rewrite a SIGINT exit code.
    #[must_use]
    pub(crate) fn captured(&self) -> Option<i32> {
        match self.received.load(Ordering::SeqCst) {
            0 => None,
            n => Some(n),
        }
    }

    /// Recover the child set even across a poisoned `Mutex`. The
    /// protected data is just a `HashSet<u32>` that any panic would
    /// have left consistent.
    fn lock_children(&self) -> MutexGuard<'_, HashSet<u32>> {
        self.children.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Track a live cargo subprocess so the handler can forward signals
    /// to it. The returned guard removes the entry on drop, including
    /// unwind paths.
    pub(crate) fn register_child(&self, pid: u32) -> ChildGuard<'_> {
        let _inserted = self.lock_children().insert(pid);
        #[cfg(unix)]
        if let Some(signal) = self.captured() {
            forward_via_kill(signal, pid);
        }
        ChildGuard { state: self, pid }
    }
}

/// RAII guard returned by [`State::register_child`].
pub(crate) struct ChildGuard<'a> {
    state: &'a State,
    pid: u32,
}

impl Drop for ChildGuard<'_> {
    fn drop(&mut self) {
        let _removed = self.state.lock_children().remove(&self.pid);
    }
}

/// Process-wide signal state, shared by [`install`] and every cargo
/// runner.
#[must_use]
pub(crate) fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(State::new)
}

/// Register every child, including startup probes, before waiting for its result.
pub(crate) fn spawn(command: &mut Command) -> std::io::Result<(Child, ChildGuard<'static>)> {
    if state().captured().is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "Lockpick was interrupted",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _command = command.process_group(0);
    }
    let child = command.spawn()?;
    let guard = state().register_child(child.id());
    Ok((child, guard))
}

/// Capture both streams while keeping the child registered until it has been reaped.
pub(crate) fn output(command: &mut Command) -> std::io::Result<Output> {
    let (child, guard) = spawn(command.stdout(Stdio::piped()).stderr(Stdio::piped()))?;
    let result = child.wait_with_output();
    drop(guard);
    result
}

/// Process exit code for a signal-aware shutdown: `128 + signum` when
/// interrupted, else `default`. Out-of-range signal numbers fall back
/// too, since shells encode killed-by-signal exits in `[129, 255]`.
#[must_use]
pub(crate) fn exit_code(captured: Option<i32>, default: u8) -> u8 {
    if let Some(sig) = captured
        && let Ok(sig) = u8::try_from(sig)
        && (1..128).contains(&sig)
    {
        128_u8.saturating_add(sig)
    } else {
        default
    }
}

/// Install the SIGINT/SIGTERM handler (no-op on non-Unix).
///
/// Spawns a background thread that drains signals forever, captures the
/// first one into `state`, and forwards every signal to all registered
/// child process groups via `kill(1)`. Setup failures silently leave the process
/// unhandled.
#[cfg(unix)]
pub(crate) fn install() {
    let Ok(mut signals) = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ]) else {
        return;
    };
    let state = state();
    let _signal_thread = std::thread::spawn(move || {
        for sig in signals.forever() {
            let _result =
                state
                    .received
                    .compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
            let pids: Vec<u32> = state.lock_children().iter().copied().collect();
            for pid in pids {
                forward_via_kill(sig, pid);
            }
        }
    });
}

/// Forward `sig` to the child process group led by `pid` via the POSIX `kill(1)` binary. Avoids a
/// libc/nix dependency just to send one signal. Errors are swallowed:
/// the child may have already exited between snapshot and call.
///
/// Argv is `kill -<signum> -- -<pgid>` to include Cargo descendants. The natural-looking
/// `-s <number>` is rejected by BSD `kill` on macOS, which expects a
/// signal *name* there.
#[cfg(unix)]
fn forward_via_kill(sig: i32, pid: u32) {
    let _result = Command::new("kill")
        .args([&format!("-{sig}"), "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
pub(crate) const fn install() {}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn exit_code_maps_signals_to_128_plus_signum() {
        assert_eq!(exit_code(None, 0), 0);
        assert_eq!(exit_code(None, 3), 3);
        assert_eq!(exit_code(Some(2_i32), 0), 130);
        assert_eq!(exit_code(Some(15_i32), 1), 143);
    }

    #[test]
    fn out_of_range_signals_fall_back_to_the_default() {
        assert_eq!(exit_code(Some(0_i32), 7), 7);
        assert_eq!(exit_code(Some(128_i32), 7), 7);
        assert_eq!(exit_code(Some(-1_i32), 7), 7);
        assert_eq!(exit_code(Some(300_i32), 7), 7);
    }

    #[test]
    fn first_captured_signal_wins() {
        let state = State::new();
        assert_eq!(state.captured(), None);
        let _result = state
            .received
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
        let _result = state
            .received
            .compare_exchange(0, 15, Ordering::SeqCst, Ordering::SeqCst);
        assert_eq!(state.captured(), Some(2_i32));
    }

    #[test]
    fn child_guard_unregisters_its_pid_on_drop() {
        let state = State::new();
        {
            let _guard = state.register_child(4242);
            assert!(state.lock_children().contains(&4242));
        }
        assert!(state.lock_children().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn late_child_registration_delivers_the_captured_signal() -> std::io::Result<()> {
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        let mut child = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let state = State::new();
        state
            .received
            .store(signal_hook::consts::SIGTERM, Ordering::SeqCst);
        let guard = state.register_child(child.id());
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > std::time::Duration::from_secs(2) {
                child.kill()?;
                let _reaped = child.wait()?;
                return Err(std::io::Error::other(
                    "late child did not receive the signal",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(status.signal(), Some(signal_hook::consts::SIGTERM));
        drop(guard);
        assert!(state.lock_children().is_empty());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn signal_installation_recovers_after_resource_exhaustion() {
        const CHILD: &str = "LOCKPICK_SIGNAL_RESOURCE_TEST";
        if std::env::var_os(CHILD).is_some() {
            let mut files = Vec::new();
            while let Ok(file) = std::fs::File::open("/dev/null") {
                files.push(file);
            }
            install();
            drop(files);
            install();
            let output = output(Command::new("sh").args(["-c", "printf recovered"])).unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, b"recovered");
            assert!(state().captured().is_none());
            let signal = Command::new("kill")
                .args(["-TERM", &std::process::id().to_string()])
                .status()
                .unwrap();
            assert!(signal.success());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while state().captured().is_none() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "signal installation did not recover"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let error = spawn(Command::new("sh").args(["-c", "exit 0"]))
                .err()
                .expect("cancelled state launched another command");
            assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
            return;
        }
        let output = crate::test_process::bounded_output(
            Command::new("sh")
                .args(["-c", "ulimit -n 64; exec \"$@\"", "signal-test"])
                .arg(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "signals::tests::signal_installation_recovers_after_resource_exhaustion",
                ])
                .env(CHILD, "1"),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn cancellation_prevents_later_process_creation() {
        const CHILD: &str = "LOCKPICK_CANCELLED_STATE_TEST";
        if std::env::var_os(CHILD).is_some() {
            let missing = tempfile::tempdir().unwrap();
            let error = output(&mut Command::new(missing.path().join("absent")))
                .expect_err("missing executable unexpectedly launched");
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            assert!(state().lock_children().is_empty());
            let executable = std::env::current_exe().unwrap();
            let first = output(Command::new(executable).arg("--list")).unwrap();
            assert!(first.status.success());
            state().received.store(2, Ordering::SeqCst);
            let error = output(&mut Command::new("must-not-be-launched"))
                .expect_err("cancelled state launched a process");
            assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
            return;
        }
        let output = crate::test_process::bounded_output(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "signals::tests::cancellation_prevents_later_process_creation",
                ])
                .env(CHILD, "1"),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
