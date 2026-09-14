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
pub(crate) struct SpawnResult {
    pub(crate) success: bool,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// Strategy that runs `cargo <sub> <args…>`. Production uses [`CargoCli`].
pub(crate) trait Runner: Send + Sync {
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
pub(crate) struct CargoCli {
    /// Isolated build directory when Cargo would overwrite this executable.
    target_dir: Option<PathBuf>,
    /// Propagated to every child as `CARGO_TERM_COLOR` so captured
    /// output matches lockpick's own stream.
    color: ColorMode,
    /// Working directory for every child. `None` inherits process cwd.
    workspace_root: Option<PathBuf>,
    /// Concrete host target for tools that do not understand Cargo's alias.
    coverage_host: Option<String>,
}

impl CargoCli {
    /// Decide whether children need `CARGO_TARGET_DIR` redirected, pin
    /// the propagated color mode, and record the workspace root.
    #[must_use]
    pub(crate) fn detect(
        color: ColorMode,
        workspace_root: Option<PathBuf>,
        target_directory: Option<&Path>,
    ) -> Self {
        Self {
            target_dir: std::env::current_exe().ok().and_then(|exe| {
                target_directory.and_then(|directory| isolated_target_dir(&exe, directory))
            }),
            color,
            workspace_root,
            coverage_host: None,
        }
    }

    /// Set the host target used by cargo-llvm-cov's direct rustc probes.
    pub(crate) fn with_coverage_host(mut self, host: Option<String>) -> Self {
        self.coverage_host = host;
        self
    }

    fn append_args(&self, command: &mut Command, sub: &str, args: &[&str]) {
        let _command = command.arg(sub);
        if sub == "llvm-cov"
            && let Some(host) = &self.coverage_host
            && let Some(position) = args
                .windows(2)
                .position(|pair| pair == ["--target", "host-tuple"])
        {
            let (before, target_and_after) = args.split_at(position);
            let _command = command
                .args(before)
                .args(["--target", host])
                .args(target_and_after.iter().skip(2));
        } else {
            let _command = command.args(args);
        }
    }

    /// Construct both captured and inherited subprocesses consistently.
    fn command(&self, sub: &str, args: &[&str], envs: &[(&str, &str)]) -> Command {
        let mut command = cargo_command();
        if let Some(root) = &self.workspace_root {
            let _command = command.current_dir(root);
        }
        self.append_args(&mut command, sub, args);
        let _command = command.env("CARGO_TERM_COLOR", self.color.as_str());
        let _command = command.envs(envs.iter().copied());
        if let Some(directory) = &self.target_dir {
            let _command = command.env("CARGO_TARGET_DIR", directory);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let _command = command.process_group(0);
        }
        command
    }

    /// Run a fix step with live output and the same cancellation behavior.
    pub(crate) fn spawn_inherited(&self, sub: &str, args: &[&str]) -> std::io::Result<bool> {
        ensure_running()?;
        let mut cmd = self.command(sub, args, &[]);
        let _command = cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
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
        ensure_running()?;
        execute(self.command(sub, args, envs))
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

fn ensure_running() -> std::io::Result<()> {
    if crate::signals::state().captured().is_some() {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "Lockpick was interrupted",
        ))
    } else {
        Ok(())
    }
}

/// Cargo metadata accounts for `CARGO_TARGET_DIR` and `build.target-dir`.
/// Keep descending until the chosen output directory excludes the running binary.
fn isolated_target_dir(executable: &Path, target_directory: &Path) -> Option<PathBuf> {
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let directory = target_directory
        .canonicalize()
        .unwrap_or_else(|_| target_directory.to_path_buf());
    if !executable.starts_with(&directory) {
        return None;
    }
    let mut isolated = directory.join("lockpick");
    while executable.starts_with(&isolated) {
        isolated = isolated.join("lockpick");
    }
    Some(isolated)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn isolation_accounts_for_custom_and_already_redirected_directories() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary
            .path()
            .canonicalize()
            .unwrap()
            .join("custom-build");
        assert!(
            isolated_target_dir(&temporary.path().join("installed/lockpick"), &directory).is_none()
        );
        assert_eq!(
            isolated_target_dir(&directory.join("debug/lockpick"), &directory),
            Some(directory.join("lockpick"))
        );
        assert_eq!(
            isolated_target_dir(&directory.join("lockpick/debug/lockpick"), &directory),
            Some(directory.join("lockpick/lockpick"))
        );
    }
}
