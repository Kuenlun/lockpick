// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Whether subprocess output captured by lockpick should carry ANSI
/// colors. Picked once per run from the report stream's state: keep
/// colors when stdout is an interactive terminal, strip them when it is
/// a pipe or when the user opted out via `NO_COLOR`
/// (<https://no-color.org>).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    Always,
    #[default]
    Never,
}

impl ColorMode {
    /// Decide the mode from the report stream's TTY state, picking up
    /// `NO_COLOR` from the environment.
    #[must_use]
    pub fn for_stdout(is_tty: bool) -> Self {
        Self::from_inputs(is_tty, no_color_env())
    }

    /// Pure half of [`Self::for_stdout`]: a TTY without `NO_COLOR` keeps
    /// colors; anything else (pipe, file, or explicit opt-out) drops
    /// them. Split out so tests can pin both branches without mutating
    /// the process environment, which would race other tests.
    #[must_use]
    pub const fn from_inputs(is_tty: bool, no_color: bool) -> Self {
        if is_tty && !no_color {
            Self::Always
        } else {
            Self::Never
        }
    }

    /// Stringification accepted by both `CARGO_TERM_COLOR` and rustfmt's
    /// `--color`, so the same value can drive cargo and rustfmt in
    /// lockstep.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

/// `NO_COLOR` is honoured when present and non-empty, matching
/// <https://no-color.org>: unset and empty both mean "color allowed".
/// `unwrap_or_default` collapses both into the same empty `OsString`,
/// so the predicate folds to a single `is_empty` check.
fn no_color_env() -> bool {
    no_color_value(&std::env::var_os("NO_COLOR").unwrap_or_default())
}

/// Pure half of [`no_color_env`], factored out so tests can pin both
/// arms (empty vs non-empty) without mutating the process environment,
/// which would race other tests.
fn no_color_value(value: &OsStr) -> bool {
    !value.is_empty()
}

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

/// Whether the active `rustc` advertises itself as a nightly build.
///
/// Nightly is what unlocks `-Z coverage-options=branch`, so this is the
/// gating signal for branch-coverage measurement. A spawn failure or
/// non-zero exit reads as "not nightly": stable is the safe fallback.
#[must_use]
pub fn is_nightly() -> bool {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| version_string_is_nightly(&String::from_utf8_lossy(&o.stdout)))
}

/// Pure parse half of [`is_nightly`], factored out so tests can pin the
/// matching rule against synthetic version strings without spawning a
/// real `rustc`.
fn version_string_is_nightly(version: &str) -> bool {
    version.contains("nightly")
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
}
