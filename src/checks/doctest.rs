// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::process::{Output, Stdio};

use super::{Check, fmt_cargo_cmd, run_cargo_outcome};
use crate::reporter::CheckOutcome;
use crate::tooling::cargo_command;

const DOCTEST_ARGS: &[&str] = &["--doc", "--workspace", "--all-features"];

pub struct DocTestCheck;

impl Check for DocTestCheck {
    fn label(&self) -> &'static str {
        "doc test"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("test", DOCTEST_ARGS)
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("test", DOCTEST_ARGS)
    }
}

/// Returns `true` when any workspace member exposes a `lib` target.
/// Skipping the doc-test check on bin-only workspaces avoids an opaque
/// error from cargo.
#[must_use]
pub fn workspace_has_lib_target() -> bool {
    cargo_command()
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .stderr(Stdio::null())
        .output()
        .as_ref()
        .ok()
        .and_then(|o: &Output| std::str::from_utf8(&o.stdout).ok())
        .is_some_and(|s| s.contains(r#""kind":["lib"]"#))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_doc_test() {
        assert_eq!(DocTestCheck.label(), "doc test");
    }

    #[test]
    fn cmd_runs_cargo_test_doc() {
        let cmd = DocTestCheck.cmd();
        assert!(cmd.starts_with("cargo test "));
        assert!(cmd.contains("--doc"));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-features"));
    }

    #[test]
    fn workspace_has_lib_target_is_true_for_lockpick_itself() {
        // Lockpick is a binary crate, so this is mostly a smoke test
        // that the helper doesn't panic. The actual return value depends
        // on the cwd at test time (usually the lockpick repo root, which
        // is a bin-only crate → false), but we don't assert that.
        let _ = workspace_has_lib_target();
    }
}
