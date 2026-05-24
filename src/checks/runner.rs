// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Strategy for spawning cargo subcommands and capturing their output.
//! [`CargoCli`] is the production [`Runner`]; alternative implementations
//! plug into the same trait without touching the check catalogue.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::tooling::{ColorMode, cargo_command};

/// Captured output of a finished cargo invocation.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Strategy that runs `cargo <sub> <args…>`. Production uses [`CargoCli`].
pub trait Runner: Send + Sync {
    /// Spawn the subcommand and capture its raw output.
    ///
    /// [`Err`] signals an OS-level launch failure; non-zero exits come
    /// back as `Ok(SpawnResult { success: false, … })`.
    fn spawn(
        &self,
        sub: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> std::io::Result<SpawnResult>;
}

/// Production [`Runner`]: shells out to the host `cargo`, scrubs
/// package-scoped env vars, and optionally redirects child builds away
/// from the parent's target directory.
///
/// Each spawn is anchored at `workspace_root` (when known) via
/// [`Command::current_dir`]. `cargo audit` only opens `./Cargo.lock`,
/// so without this anchor lockpick would disagree with itself when
/// invoked from a subdirectory. Build/clippy/fmt/machete walk up the
/// manifest tree on their own and are unaffected.
#[derive(Debug, Clone, Default)]
pub struct CargoCli {
    /// When true, children inherit `CARGO_TARGET_DIR=target/lockpick`.
    redirect_target_dir: bool,
    /// Color decision propagated to every child as `CARGO_TERM_COLOR`,
    /// so captured output matches what lockpick will print on its own
    /// stdout stream.
    color: ColorMode,
    /// Workspace root used as each child's working directory. `None`
    /// leaves children inheriting the process cwd.
    workspace_root: Option<PathBuf>,
}

impl CargoCli {
    /// Decide whether children need `CARGO_TARGET_DIR` redirected, pin
    /// the color mode propagated as `CARGO_TERM_COLOR`, and record the
    /// workspace root that anchors every spawn.
    #[must_use]
    pub fn detect(color: ColorMode, workspace_root: Option<PathBuf>) -> Self {
        Self {
            redirect_target_dir: needs_target_dir_redirect(workspace_root.as_deref()),
            color,
            workspace_root,
        }
    }
}

impl Runner for CargoCli {
    fn spawn(
        &self,
        sub: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> std::io::Result<SpawnResult> {
        let mut cmd = cargo_command();
        if let Some(root) = &self.workspace_root {
            cmd.current_dir(root);
        }
        cmd.arg(sub).args(args);
        // Set our color decision first so caller-supplied `envs` still
        // win if a check ever needs to override it for one invocation.
        cmd.env("CARGO_TERM_COLOR", self.color.as_str());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        if self.redirect_target_dir {
            cmd.env("CARGO_TARGET_DIR", "target/lockpick");
        }
        execute(cmd)
    }
}

/// Spawn the [`Command`], record its PID so the SIGINT/SIGTERM handler
/// can forward signals to it, and capture both streams. The guard
/// returned by [`crate::signals::State::register_child`] removes the
/// PID on every exit path, including the early return from `?` and
/// unwind through the explicit `drop` site.
///
/// The guard is dropped immediately after `wait_with_output` reaps the
/// child, so the window in which a recycled PID could receive a
/// forwarded signal is bounded to a few instructions. A fully race-free
/// fix would need `pidfd_send_signal` (Linux 5.3+) or its BSD equivalent
/// to address the process by handle instead of by PID.
fn execute(mut cmd: Command) -> std::io::Result<SpawnResult> {
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let guard = crate::signals::state().register_child(child.id());
    let output = child.wait_with_output();
    drop(guard);
    output.map(|out| SpawnResult {
        success: out.status.success(),
        stdout: out.stdout,
        stderr: out.stderr,
    })
}

/// Whether child cargo invocations should redirect their target dir.
///
/// Redirects only when the running binary lives under `<anchor>/target/`
/// and `CARGO_TARGET_DIR` is unset. The anchor is the workspace root
/// when known, falling back to the process cwd for non-workspace runs.
fn needs_target_dir_redirect(workspace_root: Option<&Path>) -> bool {
    if std::env::var_os("CARGO_TARGET_DIR").is_some() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let anchor = match workspace_root {
        Some(root) => root.to_path_buf(),
        None => match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(_) => return false,
        },
    };
    exe.starts_with(anchor.join("target"))
}
