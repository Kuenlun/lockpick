// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::process::{Command, Output, Stdio};

use super::{Check, run_cargo_outcome};
use crate::reporter::CheckOutcome;

pub struct DocTestCheck;

impl Check for DocTestCheck {
    fn label(&self) -> &'static str {
        "doc test"
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome("test", &["--doc", "--workspace", "--all-features"])
    }
}

/// Returns `true` when any workspace member exposes a `lib` target.
/// Skipping the doc-test check on bin-only workspaces avoids an opaque
/// error from cargo.
#[must_use]
pub fn workspace_has_lib_target() -> bool {
    Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .stderr(Stdio::null())
        .output()
        .as_ref()
        .ok()
        .and_then(|o: &Output| std::str::from_utf8(&o.stdout).ok())
        .is_some_and(|s| s.contains(r#""kind":["lib"]"#))
}
