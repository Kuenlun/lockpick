// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::process::Command;

pub const INSTALL_LLVM_COV: &str = "cargo install cargo-llvm-cov";
pub const INSTALL_MACHETE: &str = "cargo install cargo-machete";
pub const INSTALL_AUDIT: &str = "cargo install cargo-audit";

/// Detect a third-party cargo subcommand by looking for its `cargo-<name>`
/// binary on `PATH`. Probing via `cargo <name> --version` would spawn the
/// subcommand and is brittle: cargo-machete in particular flips its argv
/// parser when `CARGO_PKG_NAME` is set (the case under `cargo run`),
/// reading "machete" and "--version" as paths instead of the subcommand
/// and a flag, and reporting itself as missing.
fn has_cargo_subcommand(subcommand: &str) -> bool {
    let name = format!("cargo-{subcommand}");
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        if dir.join(&name).is_file() {
            return true;
        }
        #[cfg(windows)]
        for ext in ["exe", "cmd", "bat"] {
            if dir.join(format!("{name}.{ext}")).is_file() {
                return true;
            }
        }
        false
    })
}

/// Returns `true` for env var names that cargo sets to describe the *current*
/// package's build and that must be stripped before spawning a child
/// `cargo` invocation. Inheriting them poisons tools like cargo-machete,
/// which interpret `CARGO_PKG_NAME` as "I'm being run from inside a build"
/// and switch their argv parser into a positional-paths-only mode.
fn should_scrub_cargo_env(key: &str) -> bool {
    key.starts_with("CARGO_PKG_")
        || key.starts_with("CARGO_BIN_")
        || key.starts_with("CARGO_CRATE_")
        || matches!(
            key,
            "CARGO_MANIFEST_DIR" | "CARGO_MANIFEST_PATH" | "CARGO_PRIMARY_PACKAGE"
        )
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
    /// Probe the host for every tool lockpick knows about.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            llvm_cov: has_cargo_subcommand("llvm-cov"),
            nextest: has_cargo_subcommand("nextest"),
            machete: has_cargo_subcommand("machete"),
            audit: has_cargo_subcommand("audit"),
        }
    }

    /// Construct a snapshot with every tool reported as present. Useful
    /// for tests that don't want to be affected by what's installed.
    #[cfg(test)]
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
mod tests {
    use super::*;

    #[test]
    fn install_hints_are_non_empty() {
        for hint in [INSTALL_LLVM_COV, INSTALL_MACHETE, INSTALL_AUDIT] {
            assert!(hint.starts_with("cargo install "), "got: {hint}");
            assert!(hint.len() > "cargo install ".len());
        }
    }

    #[test]
    fn detect_does_not_panic_and_returns_bools() {
        let t = Toolchain::detect();
        // Smoke-test: the booleans are well-defined regardless of host state.
        let _ = (t.llvm_cov, t.nextest, t.machete, t.audit);
    }

    #[test]
    fn unknown_subcommand_is_not_detected() {
        assert!(!has_cargo_subcommand(
            "definitely-not-a-real-cargo-subcommand"
        ));
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

    #[test]
    fn all_present_helper_reports_every_tool() {
        let t = Toolchain::all_present();
        assert!(t.llvm_cov && t.nextest && t.machete && t.audit);
    }

    #[test]
    fn default_reports_no_tool() {
        let t = Toolchain::default();
        assert!(!t.llvm_cov && !t.nextest && !t.machete && !t.audit);
    }
}
