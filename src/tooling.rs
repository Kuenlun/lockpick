// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Check `PATH` for a `cargo-<subcommand>` binary.
///
/// Filesystem-only probe by design: spawning `cargo <name> --version`
/// would flip cargo-machete's argv parser into positional-paths mode
/// under `CARGO_PKG_NAME` (i.e. when invoked from `cargo run`) and
/// report itself as missing.
fn has_cargo_subcommand_in(path_env: Option<&OsStr>, subcommand: &str) -> bool {
    path_env.is_some_and(|path| {
        let name = format!("cargo-{subcommand}");
        std::env::split_paths(path).any(|dir| contains_executable(&dir, &name))
    })
}

fn contains_executable(dir: &Path, name: &str) -> bool {
    if dir.join(name).is_file() {
        return true;
    }
    #[cfg(windows)]
    for ext in ["exe", "cmd", "bat"] {
        if dir.join(format!("{name}.{ext}")).is_file() {
            return true;
        }
    }
    false
}

/// Env var prefixes that describe the *current* package's build. Must
/// be stripped before spawning child cargo invocations or cargo-machete
/// flips into positional-paths-only argv parsing.
const SCRUB_PREFIXES: &[&str] = &["CARGO_PKG_", "CARGO_BIN_", "CARGO_CRATE_"];
const SCRUB_EXACT: &[&str] = &[
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "CARGO_PRIMARY_PACKAGE",
];

fn should_scrub_cargo_env(key: &str) -> bool {
    SCRUB_PREFIXES.iter().any(|p| key.starts_with(p)) || SCRUB_EXACT.contains(&key)
}

/// Build a [`Command`] for `cargo` with package-scoped env vars
/// scrubbed so they cannot leak from `cargo run` into subcommands.
#[must_use]
pub fn cargo_command() -> Command {
    let mut cmd = Command::new("cargo");
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(should_scrub_cargo_env) {
            cmd.env_remove(&key);
        }
    }
    cmd
}

/// Optional cargo subcommand lockpick can drive. Each variant resolves
/// to a `cargo-<binary>` lookup on `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    LlvmCov,
    Nextest,
    Machete,
    Audit,
}

impl Tool {
    /// Cargo plugin suffix, e.g. `Tool::LlvmCov → "llvm-cov"`.
    const fn subcommand(self) -> &'static str {
        match self {
            Self::LlvmCov => "llvm-cov",
            Self::Nextest => "nextest",
            Self::Machete => "machete",
            Self::Audit => "audit",
        }
    }
}

/// Single source of truth for every [`Tool`] variant.
const ALL_TOOLS: &[Tool] = &[Tool::LlvmCov, Tool::Nextest, Tool::Machete, Tool::Audit];

/// Snapshot of optional cargo subcommands installed on the host.
#[derive(Debug, Clone, Default)]
pub struct Toolchain {
    present: HashSet<Tool>,
}

impl Toolchain {
    /// Probe the host `PATH` for every known tool.
    #[cfg_attr(test, allow(dead_code))]
    #[must_use]
    pub fn detect() -> Self {
        let path = std::env::var_os("PATH");
        let present = ALL_TOOLS
            .iter()
            .copied()
            .filter(|t| has_cargo_subcommand_in(path.as_deref(), t.subcommand()))
            .collect();
        Self { present }
    }

    /// Whether `tool` is installed.
    #[must_use]
    pub fn has(&self, tool: Tool) -> bool {
        self.present.contains(&tool)
    }

    /// Snapshot with every tool present.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    #[must_use]
    pub fn all_present() -> Self {
        Self {
            present: ALL_TOOLS.iter().copied().collect(),
        }
    }

    /// Return `self` minus `tool`.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    #[must_use]
    pub fn without(mut self, tool: Tool) -> Self {
        self.present.remove(&tool);
        self
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn has_cargo_subcommand_in_returns_false_when_path_is_unset() {
        assert!(!has_cargo_subcommand_in(None, "anything"));
    }

    #[test]
    fn has_cargo_subcommand_in_returns_false_when_path_is_empty() {
        let empty = OsString::new();
        assert!(!has_cargo_subcommand_in(
            Some(empty.as_os_str()),
            "anything"
        ));
    }

    #[test]
    fn has_cargo_subcommand_in_finds_executable_on_synthesised_path() {
        let dir = std::env::temp_dir().join(format!(
            "lockpick_tooling_{pid}_{nanos}",
            pid = std::process::id(),
            nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("cargo-lockpicktest");
        std::fs::write(&fake, b"#!/bin/sh\nexit 0\n").unwrap();
        let path_value = OsString::from(dir.as_os_str());
        assert!(has_cargo_subcommand_in(
            Some(path_value.as_os_str()),
            "lockpicktest"
        ));
        assert!(!has_cargo_subcommand_in(
            Some(path_value.as_os_str()),
            "definitely-absent"
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn contains_executable_returns_false_when_path_does_not_exist() {
        let nonexistent = Path::new("/definitely/does/not/exist");
        assert!(!contains_executable(nonexistent, "cargo-x"));
    }

    #[cfg(windows)]
    #[test]
    fn contains_executable_finds_exe_extension_on_windows() {
        let dir = std::env::temp_dir().join(format!(
            "lockpick_tooling_exe_{pid}_{nanos}",
            pid = std::process::id(),
            nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("cargo-lockpicktestext.exe");
        std::fs::write(&fake, b"").unwrap();
        assert!(contains_executable(&dir, "cargo-lockpicktestext"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_scrub_cargo_env_targets_package_scoped_vars() {
        assert!(should_scrub_cargo_env("CARGO_PKG_NAME"));
        assert!(should_scrub_cargo_env("CARGO_PKG_VERSION"));
        assert!(should_scrub_cargo_env("CARGO_BIN_NAME"));
        assert!(should_scrub_cargo_env("CARGO_BIN_EXE_lockpick"));
        assert!(should_scrub_cargo_env("CARGO_CRATE_NAME"));
        assert!(should_scrub_cargo_env("CARGO_MANIFEST_DIR"));
        assert!(should_scrub_cargo_env("CARGO_MANIFEST_PATH"));
        assert!(should_scrub_cargo_env("CARGO_PRIMARY_PACKAGE"));
    }

    #[test]
    fn should_scrub_cargo_env_preserves_global_vars() {
        assert!(!should_scrub_cargo_env("CARGO"));
        assert!(!should_scrub_cargo_env("CARGO_HOME"));
        assert!(!should_scrub_cargo_env("CARGO_TARGET_DIR"));
        assert!(!should_scrub_cargo_env("PATH"));
        assert!(!should_scrub_cargo_env("RUSTUP_TOOLCHAIN"));
    }
}
