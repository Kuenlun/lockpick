// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Strategy for spawning cargo subcommands and capturing their output.
//! [`CargoCli`] is the production [`Runner`]. Alternative
//! implementations plug into the same trait without touching the
//! check catalogue.

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
    /// [`Err`] signals an OS-level launch failure. Non-zero exits come
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
/// [`Command::current_dir`] so `cargo audit`, which only opens
/// `./Cargo.lock`, agrees with lockpick from any subdirectory. Other
/// checks walk up the manifest tree on their own and are unaffected.
#[derive(Debug, Clone, Default)]
pub struct CargoCli {
    /// When true, children inherit `CARGO_TARGET_DIR=target/lockpick`.
    redirect_target_dir: bool,
    /// Propagated to every child as `CARGO_TERM_COLOR` so captured
    /// output matches lockpick's own stream.
    color: ColorMode,
    /// Working directory for every child. `None` inherits process cwd.
    workspace_root: Option<PathBuf>,
}

impl CargoCli {
    /// Decide whether children need `CARGO_TARGET_DIR` redirected, pin
    /// the propagated color mode, and record the workspace root.
    #[must_use]
    pub fn detect(color: ColorMode, workspace_root: Option<PathBuf>) -> Self {
        Self {
            redirect_target_dir: needs_target_dir_redirect(workspace_root.as_deref()),
            color,
            workspace_root,
        }
    }

    /// Spawn `cargo <sub> <args…>` with both streams inherited so the
    /// user sees live output. Same anchoring, env scrubbing and signal
    /// forwarding as [`Runner::spawn`], minus the capture.
    pub fn spawn_inherited(&self, sub: &str, args: &[&str]) -> std::io::Result<bool> {
        let mut cmd = cargo_command();
        if let Some(root) = &self.workspace_root {
            cmd.current_dir(root);
        }
        cmd.arg(sub).args(args);
        cmd.env("CARGO_TERM_COLOR", self.color.as_str());
        if self.redirect_target_dir {
            cmd.env("CARGO_TARGET_DIR", "target/lockpick");
        }
        cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        let mut child = cmd.spawn()?;
        let guard = crate::signals::state().register_child(child.id());
        let status = child.wait();
        drop(guard);
        status.map(|s| s.success())
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
        // Set color first so caller-supplied `envs` can still override
        // it per-invocation.
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

/// Spawn the [`Command`], register its PID so the SIGINT/SIGTERM
/// handler can forward signals to it, and capture both streams. The
/// guard is dropped after `wait_with_output` reaps the child, so the
/// PID-recycling race window is bounded to a handful of instructions
/// (a fully race-free fix would need `pidfd_send_signal` or BSD's
/// equivalent).
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
/// Redirects when the running binary lives under `<anchor>/target/`
/// and `CARGO_TARGET_DIR` is unset. The anchor is `workspace_root`
/// when known, otherwise the process cwd.
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
