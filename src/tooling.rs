// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Whether captured subprocess output should carry ANSI colors. Keep
/// them when stdout is an interactive terminal, strip them on a pipe or
/// when `NO_COLOR` is set (<https://no-color.org>).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ColorMode {
    Always,
    #[default]
    Never,
}

impl ColorMode {
    /// Decide the mode from the report stream's TTY state and the
    /// `NO_COLOR` env var.
    #[must_use]
    pub(crate) fn for_stdout(is_tty: bool) -> Self {
        if is_tty && !no_color_env() {
            Self::Always
        } else {
            Self::Never
        }
    }

    /// Form accepted by both `CARGO_TERM_COLOR` and rustfmt's `--color`,
    /// so cargo and rustfmt stay in lockstep.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

/// `NO_COLOR` is honoured when present and non-empty, matching
/// <https://no-color.org>: unset and empty both mean "color allowed".
fn no_color_env() -> bool {
    !std::env::var_os("NO_COLOR").unwrap_or_default().is_empty()
}

/// Check `PATH` for a `cargo-<subcommand>` binary.
///
/// Filesystem-only by design: spawning `cargo <name> --version` would
/// flip cargo-machete's argv parser into positional-paths mode under a
/// leaked `CARGO_PKG_NAME` (i.e. when invoked from `cargo run`) and
/// report itself as missing.
fn has_cargo_subcommand_in(path_env: Option<&OsStr>, subcommand: &str) -> bool {
    path_env.is_some_and(|path| {
        let name = format!("cargo-{subcommand}");
        std::env::split_paths(path).any(|dir| contains_executable(&dir, &name))
    })
}

fn contains_executable(dir: &Path, name: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(dir.join(name))
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    if dir.join(name).is_file() {
        return true;
    }
    #[cfg(windows)]
    for ext in ["exe", "cmd", "bat"] {
        if dir.join(format!("{name}.{ext}")).is_file() {
            return true;
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
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
pub(crate) fn cargo_command() -> Command {
    let mut cmd = Command::new("cargo");
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(should_scrub_cargo_env) {
            let _command = cmd.env_remove(&key);
        }
    }
    cmd
}

/// Whether the active `rustc` advertises itself as a nightly build.
/// Used to gate `-Z coverage-options=branch`. Spawn failure or non-zero
/// exit reads as "not nightly": stable is the safe fallback.
#[must_use]
pub(crate) fn is_nightly() -> bool {
    crate::signals::output(
        rustc_command()
            .arg("--version")
            .stdin(std::process::Stdio::null()),
    )
    .ok()
    .filter(|o| o.status.success())
    .is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains("nightly"))
}

/// Resolve Cargo's host alias before passing a target to cargo-llvm-cov.
pub(crate) fn coverage_host() -> Result<String, crate::error::LockpickError> {
    let output = crate::signals::output(
        rustc_command()
            .args(["--print", "host-tuple"])
            .stdin(std::process::Stdio::null()),
    )
    .map_err(|error| {
        crate::error::LockpickError::Configuration(format!(
            "could not query the coverage host target: {error}"
        ))
    })?;
    let host = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success()
        || host.is_empty()
        || host.starts_with('-')
        || host.chars().any(char::is_whitespace)
    {
        return Err(crate::error::LockpickError::Configuration(format!(
            "could not determine the coverage host target: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(host)
}

/// Both compiler probes honor Cargo's explicit compiler override.
fn rustc_command() -> Command {
    Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
}

/// Optional cargo subcommand lockpick can drive. Each variant resolves
/// to a `cargo-<binary>` lookup on `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Tool {
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
pub(crate) struct Toolchain {
    present: HashSet<Tool>,
}

impl Toolchain {
    /// Probe the host `PATH` for every known tool.
    #[must_use]
    pub(crate) fn detect() -> Self {
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
    pub(crate) fn has(&self, tool: Tool) -> bool {
        self.present.contains(&tool)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn color_mode_is_never_on_a_non_tty_regardless_of_env() {
        // The TTY check short-circuits before NO_COLOR is consulted, so
        // this stays deterministic under any test environment.
        assert_eq!(ColorMode::for_stdout(false), ColorMode::Never);
    }

    #[test]
    fn color_mode_renders_cargo_compatible_strings() {
        assert_eq!(ColorMode::Always.as_str(), "always");
        assert_eq!(ColorMode::Never.as_str(), "never");
    }

    #[test]
    fn package_scoped_env_vars_are_scrubbed_and_global_ones_survive() {
        for key in [
            "CARGO_PKG_NAME",
            "CARGO_BIN_NAME",
            "CARGO_CRATE_NAME",
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_PRIMARY_PACKAGE",
        ] {
            assert!(should_scrub_cargo_env(key), "{key} must be scrubbed");
        }
        for key in ["CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_TERM_COLOR", "PATH"] {
            assert!(!should_scrub_cargo_env(key), "{key} must survive");
        }
    }

    #[test]
    fn subcommand_lookup_scans_path_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cargo-zzz"), "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(!has_cargo_subcommand_in(
                Some(dir.path().as_os_str()),
                "zzz"
            ));
            std::fs::set_permissions(
                dir.path().join("cargo-zzz"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert!(has_cargo_subcommand_in(Some(&path), "zzz"));
        assert!(!has_cargo_subcommand_in(Some(&path), "absent"));
        assert!(!has_cargo_subcommand_in(None, "zzz"));
    }

    #[test]
    fn every_tool_maps_to_its_cargo_binary_suffix() {
        let subcommands: Vec<&str> = ALL_TOOLS.iter().map(|t| t.subcommand()).collect();
        assert_eq!(subcommands, ["llvm-cov", "nextest", "machete", "audit"]);
    }

    #[test]
    fn default_toolchain_reports_every_tool_absent() {
        let toolchain = Toolchain::default();
        for tool in ALL_TOOLS {
            assert!(!toolchain.has(*tool));
        }
    }

    #[test]
    fn plugin_lookup_rejects_directories_and_supports_windows_extensions() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("cargo-folder")).unwrap();
        assert!(!contains_executable(directory.path(), "cargo-folder"));
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat"] {
            let path = directory.path().join(format!("cargo-plugin.{extension}"));
            std::fs::write(&path, "").unwrap();
            assert!(contains_executable(directory.path(), "cargo-plugin"));
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn automatic_color_obeys_no_color_without_affecting_explicit_choices() {
        const CHILD: &str = "LOCKPICK_COLOR_TEST";
        if let Ok(expected) = std::env::var(CHILD) {
            let mode = <crate::cli::Cli as clap::Parser>::parse_from(["lockpick"]).color_mode(true);
            assert_eq!(mode.as_str(), expected);
            return;
        }
        for value in [None, Some(""), Some("1")] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            let _command = command
                .args(["--exact", "tooling::tests::automatic_color_obeys_no_color_without_affecting_explicit_choices"])
                .env(CHILD, if value == Some("1") { "never" } else { "always" })
                .env_remove("NO_COLOR");
            if let Some(value) = value {
                let _command = command.env("NO_COLOR", value);
            }
            let output = crate::test_process::bounded_output(&mut command).unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}
