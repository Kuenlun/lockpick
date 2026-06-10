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
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// Process-wide registry of in-flight cargo child PIDs plus the latest
/// captured signal number.
pub struct State {
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
    pub fn captured(&self) -> Option<i32> {
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
    pub fn register_child(&self, pid: u32) -> ChildGuard<'_> {
        self.lock_children().insert(pid);
        ChildGuard { state: self, pid }
    }
}

/// RAII guard returned by [`State::register_child`].
pub struct ChildGuard<'a> {
    state: &'a State,
    pid: u32,
}

impl Drop for ChildGuard<'_> {
    fn drop(&mut self) {
        self.state.lock_children().remove(&self.pid);
    }
}

/// Process-wide signal state, shared by [`install`] and every cargo
/// runner.
#[must_use]
pub fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(State::new)
}

/// Process exit code for a signal-aware shutdown: `128 + signum` when
/// interrupted, else `default`. Out-of-range signal numbers fall back
/// too, since shells encode killed-by-signal exits in `[129, 255]`.
#[must_use]
pub fn exit_code(captured: Option<i32>, default: u8) -> u8 {
    if let Some(sig) = captured
        && let Ok(sig) = u8::try_from(sig)
        && (1..128).contains(&sig)
    {
        128 + sig
    } else {
        default
    }
}

/// Install the SIGINT/SIGTERM handler (no-op on non-Unix).
///
/// Spawns a background thread that drains signals forever, captures the
/// first one into `state`, and forwards every signal to all registered
/// child PIDs via `kill(1)`. Setup failures silently leave the process
/// unhandled.
#[cfg(unix)]
pub fn install() {
    let Ok(mut signals) = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ]) else {
        return;
    };
    let state = state();
    std::thread::spawn(move || {
        for sig in signals.forever() {
            let _ = state
                .received
                .compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
            let pids: Vec<u32> = state.lock_children().iter().copied().collect();
            for pid in pids {
                forward_via_kill(sig, pid);
            }
        }
    });
}

/// Forward `sig` to `pid` via the POSIX `kill(1)` binary. Avoids a
/// libc/nix dependency just to send one signal. Errors are swallowed:
/// the child may have already exited between snapshot and call.
///
/// Argv is the XSI form `kill -<signum> <pid>`. The natural-looking
/// `-s <number>` is rejected by BSD `kill` on macOS, which expects a
/// signal *name* there.
#[cfg(unix)]
fn forward_via_kill(sig: i32, pid: u32) {
    use std::process::{Command, Stdio};

    let _ = Command::new("kill")
        .args([&format!("-{sig}"), &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
pub const fn install() {}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn exit_code_maps_signals_to_128_plus_signum() {
        assert_eq!(exit_code(None, 0), 0);
        assert_eq!(exit_code(None, 3), 3);
        assert_eq!(exit_code(Some(2), 0), 130);
        assert_eq!(exit_code(Some(15), 1), 143);
    }

    #[test]
    fn out_of_range_signals_fall_back_to_the_default() {
        assert_eq!(exit_code(Some(0), 7), 7);
        assert_eq!(exit_code(Some(128), 7), 7);
        assert_eq!(exit_code(Some(-1), 7), 7);
        assert_eq!(exit_code(Some(300), 7), 7);
    }

    #[test]
    fn first_captured_signal_wins() {
        let state = State::new();
        assert_eq!(state.captured(), None);
        let _ = state
            .received
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
        let _ = state
            .received
            .compare_exchange(0, 15, Ordering::SeqCst, Ordering::SeqCst);
        assert_eq!(state.captured(), Some(2));
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
}
