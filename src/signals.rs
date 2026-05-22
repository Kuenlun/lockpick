// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! SIGINT/SIGTERM capture and child-process signal forwarding.
//!
//! Without this, a `kill -INT` mid-pipeline kills the cargo subprocesses
//! (via the terminal's foreground process group) and lockpick interprets
//! their non-zero exits as ordinary check failures, returning `1` instead
//! of the canonical `128 + signum`. The handler also forwards the signal
//! to every live child so the explicit `kill -INT $lockpick_pid` case,
//! where the terminal does not broadcast, no longer leaves cargo running
//! detached after lockpick winds down.

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

    /// Signal that interrupted the run, or `None` if it ran to completion.
    /// First signal wins so a follow-up SIGTERM cannot rewrite a SIGINT
    /// exit code already on the wire.
    #[must_use]
    pub fn captured(&self) -> Option<i32> {
        match self.received.load(Ordering::SeqCst) {
            0 => None,
            n => Some(n),
        }
    }

    /// Recover the child set even across a poisoned `Mutex`. Lock poison
    /// here would only follow a panic inside a child-tracking critical
    /// section, where the protected data is still consistent.
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

/// Compute the process exit code for a signal-aware shutdown.
///
/// When the user interrupted us, return `128 + signum` so the shell
/// reports the canonical value; otherwise fall back to `default`. Out-
/// of-range signal numbers fall back too, since shells encode killed-
/// by-signal exits in `[129, 255]`.
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

/// Drain a stream of signal numbers, capturing the first one in `state`
/// and forwarding every signal to all registered child PIDs.
#[cfg(unix)]
pub fn process_signals(
    signals: impl IntoIterator<Item = i32>,
    state: &State,
    mut forward: impl FnMut(i32, u32),
) {
    for sig in signals {
        let _ = state
            .received
            .compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst);
        let pids: Vec<u32> = state.lock_children().iter().copied().collect();
        for pid in pids {
            forward(sig, pid);
        }
    }
}

/// Install the SIGINT/SIGTERM handler. On non-Unix targets, a no-op.
///
/// The owned `Signals` value is moved into a closure that produces one
/// `i32` per call to `next()`, keeping the signal source `'static`
/// without a hand-rolled iterator wrapper.
#[cfg(unix)]
pub fn install() {
    let signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ])
    .map(|mut s| std::iter::from_fn(move || s.forever().next()));
    let _ = spawn_handler(state(), signals, forward_via_kill);
}

/// Forward `sig` to `pid` by shelling out to `kill(1)`. The binary is
/// part of every POSIX install, so this avoids pulling in a libc / nix /
/// rustix dependency just to send a single signal. Errors are swallowed
/// because the child may have already exited between the registry
/// snapshot and the `kill` call.
///
/// The argv is `kill -<signum> <pid>`, the XSI form. GNU kill also
/// accepts `-s <number>`, but BSD kill on macOS treats `-s` as taking
/// a signal *name* and silently rejects `-s 2`, so the natural-looking
/// alternative would leave macOS children unsignalled.
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

/// Spawn the signal-handler thread for an already-constructed source.
/// Returns `Some(handle)` when the source was `Ok` and the thread was
/// launched; `None` when the source failed (signal-hook setup denied
/// by the OS, typically on resource exhaustion).
#[cfg(unix)]
fn spawn_handler<I, F>(
    state: &'static State,
    signals: std::io::Result<I>,
    forward: F,
) -> Option<std::thread::JoinHandle<()>>
where
    I: IntoIterator<Item = i32> + Send + 'static,
    F: FnMut(i32, u32) + Send + 'static,
{
    let iter = signals.ok()?;
    Some(std::thread::spawn(move || {
        process_signals(iter, state, forward);
    }))
}
