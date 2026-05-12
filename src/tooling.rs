// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

pub const INSTALL_LLVM_COV: &str = "cargo install cargo-llvm-cov";
pub const INSTALL_MACHETE: &str = "cargo install cargo-machete";
pub const INSTALL_AUDIT: &str = "cargo install cargo-audit";

/// Look for a `cargo-<subcommand>` binary on the supplied `PATH`. Probing
/// via `cargo <name> --version` would spawn the subcommand and is brittle:
/// cargo-machete in particular flips its argv parser when `CARGO_PKG_NAME`
/// is set (the case under `cargo run`), reading "machete" and "--version"
/// as paths instead of the subcommand and a flag, and reporting itself as
/// missing. Returns `false` when `path_env` is `None` to mirror the real
/// behaviour of an unset PATH.
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

/// Package-scoped env var prefixes whose values describe the *current*
/// package's build and that must be stripped before spawning child cargo
/// invocations. Inheriting them poisons tools like cargo-machete, which
/// interpret `CARGO_PKG_NAME` as "I'm being run from inside a build" and
/// switch their argv parser into a positional-paths-only mode.
const SCRUB_PREFIXES: &[&str] = &["CARGO_PKG_", "CARGO_BIN_", "CARGO_CRATE_"];
const SCRUB_EXACT: &[&str] = &[
    "CARGO_MANIFEST_DIR",
    "CARGO_MANIFEST_PATH",
    "CARGO_PRIMARY_PACKAGE",
];

fn should_scrub_cargo_env(key: &str) -> bool {
    SCRUB_PREFIXES.iter().any(|p| key.starts_with(p)) || SCRUB_EXACT.contains(&key)
}

/// Builder for child `cargo` invocations with a hygienic environment.
/// Use this everywhere lockpick spawns cargo so that package-scoped vars
/// from lockpick's own build (when run via `cargo run`) don't leak into
/// subcommands.
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

/// Snapshot of which optional cargo subcommands are installed on the host.
/// Constructed once at the start of a run via [`Toolchain::detect`]; passed
/// into the rest of the pipeline so unit tests can substitute fake values.
/// The four bool fields each represent a distinct, independent capability,
/// so collapsing them into bit flags would only hurt readability.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Toolchain {
    pub llvm_cov: bool,
    pub nextest: bool,
    pub machete: bool,
    pub audit: bool,
}

impl Toolchain {
    /// Probe the host for every tool lockpick knows about. Only `runner::run`
    /// calls this; unit tests construct fixed `Toolchain` snapshots instead,
    /// so the dead-code lint has to be silenced in test builds.
    #[cfg_attr(test, allow(dead_code))]
    #[must_use]
    pub fn detect() -> Self {
        let path = std::env::var_os("PATH");
        let probe = |sub| has_cargo_subcommand_in(path.as_deref(), sub);
        Self {
            llvm_cov: probe("llvm-cov"),
            nextest: probe("nextest"),
            machete: probe("machete"),
            audit: probe("audit"),
        }
    }

    /// Construct a snapshot with every tool reported as present. Useful
    /// for tests that don't want to be affected by what's installed.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    #[must_use]
    pub const fn all_present() -> Self {
        Self {
            llvm_cov: true,
            nextest: true,
            machete: true,
            audit: true,
        }
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
        // No need to mark executable: the lookup only checks `is_file()`.
        let path_value = OsString::from(dir.as_os_str());
        assert!(has_cargo_subcommand_in(
            Some(path_value.as_os_str()),
            "lockpicktest"
        ));
        // And the negative case for the same PATH:
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
        // CARGO and CARGO_HOME describe the toolchain itself, not this
        // package — children need them to find the right cargo/registry.
        assert!(!should_scrub_cargo_env("CARGO"));
        assert!(!should_scrub_cargo_env("CARGO_HOME"));
        assert!(!should_scrub_cargo_env("CARGO_TARGET_DIR"));
        assert!(!should_scrub_cargo_env("PATH"));
        assert!(!should_scrub_cargo_env("RUSTUP_TOOLCHAIN"));
    }
}
