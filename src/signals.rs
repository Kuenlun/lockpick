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
///
/// Pure orchestrator: the signal source, the state and the forwarder are
/// all injected so tests can exercise both branches without installing
/// real handlers or spawning subprocesses.
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
/// `i32` per call to `next()`. This keeps the signal source `'static`
/// without a hand-rolled iterator wrapper, and lets [`spawn_handler`]
/// stay generic so both `Result` arms are exercised from unit tests.
#[cfg(all(unix, not(test)))]
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
#[cfg(all(unix, not(test)))]
fn forward_via_kill(sig: i32, pid: u32) {
    use std::process::{Command, Stdio};

    let _ = Command::new("kill")
        .args(["-s", &sig.to_string(), &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(all(unix, not(test))))]
pub const fn install() {}

/// Spawn the signal-handler thread for an already-constructed source.
/// Returns `Some(handle)` when the source was `Ok` and the thread was
/// launched; `None` when the source failed (signal-hook setup denied
/// by the OS, typically on resource exhaustion).
///
/// State is injected so unit tests can drive both arms against a
/// leaked-on-the-heap `State` without polluting the production
/// singleton's `received` slot.
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn captured_is_none_before_any_signal_arrives() {
        let state = State::new();
        assert!(state.captured().is_none());
    }

    #[test]
    fn captured_returns_the_first_signal_after_process_signals_drains_it() {
        let state = State::new();
        process_signals([2], &state, |_, _| {});
        assert_eq!(state.captured(), Some(2));
    }

    #[test]
    fn first_signal_wins_against_subsequent_ones() {
        let state = State::new();
        process_signals([2, 15], &state, |_, _| {});
        assert_eq!(
            state.captured(),
            Some(2),
            "subsequent signals must not rewrite the captured one",
        );
    }

    #[test]
    fn process_signals_forwards_each_signal_to_every_registered_pid() {
        let state = State::new();
        let _g1 = state.register_child(100);
        let _g2 = state.register_child(200);
        let mut sent: Vec<(i32, u32)> = Vec::new();
        process_signals([2], &state, |sig, pid| sent.push((sig, pid)));
        sent.sort_unstable();
        assert_eq!(sent, vec![(2, 100), (2, 200)]);
    }

    #[test]
    fn process_signals_forwards_each_signal_separately() {
        let state = State::new();
        let _g = state.register_child(42);
        let mut sent: Vec<(i32, u32)> = Vec::new();
        process_signals([2, 15], &state, |sig, pid| sent.push((sig, pid)));
        assert_eq!(sent, vec![(2, 42), (15, 42)]);
    }

    #[test]
    fn child_guard_unregisters_on_drop_even_if_the_run_panics() {
        let state = State::new();
        {
            let _g = state.register_child(7);
            assert!(state.lock_children().contains(&7));
        }
        assert!(
            !state.lock_children().contains(&7),
            "guard must remove the PID when it goes out of scope",
        );
    }

    #[test]
    fn process_signals_on_empty_registry_skips_the_forward_callback() {
        let state = State::new();
        let mut called = false;
        process_signals([2], &state, |_, _| called = true);
        assert!(!called, "no children, no forwards");
    }

    #[test]
    fn exit_code_returns_default_when_no_signal_was_captured() {
        assert_eq!(exit_code(None, 0), 0);
        assert_eq!(exit_code(None, 1), 1);
        assert_eq!(exit_code(None, 42), 42);
    }

    #[test]
    fn exit_code_returns_one_twenty_eight_plus_signum_for_sigint_and_sigterm() {
        assert_eq!(exit_code(Some(2), 1), 130, "SIGINT must map to 130");
        assert_eq!(exit_code(Some(15), 1), 143, "SIGTERM must map to 143");
    }

    #[test]
    fn exit_code_falls_back_for_signal_numbers_outside_the_shell_range() {
        // Shells encode killed-by-signal exits in [129, 255], so values
        // outside `(0, 128)` are not addressable as exit codes and fall
        // back to the default rather than wrapping into garbage.
        assert_eq!(exit_code(Some(0), 7), 7);
        assert_eq!(exit_code(Some(-1), 7), 7);
        assert_eq!(exit_code(Some(128), 7), 7);
        assert_eq!(exit_code(Some(999), 7), 7);
    }

    #[test]
    fn state_singleton_is_addressable() {
        // The global is built once and shared; calling `state()` twice
        // must hand back the same address.
        let a = std::ptr::from_ref(state());
        let b = std::ptr::from_ref(state());
        assert_eq!(a, b);
    }

    #[test]
    fn install_on_unsupported_targets_is_a_no_op_that_returns() {
        // `install` is `()` on non-Unix and on test builds. Calling it
        // here pins the stub branch so it stays compiled and covered.
        install();
    }

    /// Drive both arms of `spawn_handler` from a single generic
    /// instantiation: same iterator type and same forwarder fn pointer
    /// on both call sites, so the monomorphised closure body is reached
    /// at least once (via the `Ok` branch) and llvm-cov records the
    /// instantiation as live. Without this, the `Err`-only call site
    /// generates a dead closure body that masquerades as uncovered code.
    #[cfg(unix)]
    #[test]
    fn spawn_handler_covers_both_arms_via_one_generic_instantiation() {
        use std::sync::atomic::{AtomicBool, Ordering};

        // Module-scoped so it has fn-pointer identity (a closure would
        // mint a fresh anonymous type per call site, forcing a second
        // instantiation of `spawn_handler`).
        static FIRED: AtomicBool = AtomicBool::new(false);
        fn record(_sig: i32, _pid: u32) {
            FIRED.store(true, Ordering::SeqCst);
        }
        FIRED.store(false, Ordering::SeqCst);

        // Dedicated `'static` state for the test: the production
        // singleton must stay pristine so any future test asserting
        // `state().captured().is_none()` is not contaminated by this
        // one's SIGINT.
        let test_state: &'static State = Box::leak(Box::new(State::new()));

        let err = spawn_handler::<std::iter::Once<i32>, fn(i32, u32)>(
            test_state,
            Err(std::io::Error::other("simulated install failure")),
            record,
        );
        assert!(err.is_none(), "Err must short-circuit before the spawn");

        // Register a sentinel PID so `process_signals` invokes the
        // forwarder for the one signal we feed it.
        let _g = test_state.register_child(424_242);
        let ok = spawn_handler::<std::iter::Once<i32>, fn(i32, u32)>(
            test_state,
            Ok(std::iter::once(2)),
            record,
        )
        .expect("Ok must return the join handle");
        ok.join().expect("handler thread must not panic");
        assert!(
            FIRED.load(Ordering::SeqCst),
            "spawned handler must forward the signal to registered children",
        );
        assert_eq!(
            test_state.captured(),
            Some(2),
            "the test-only state must record the signal we just drained",
        );
    }
}
