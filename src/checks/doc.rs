// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Documentation build with rustdoc warnings escalated to errors.
//! Catches broken intra-doc links, unresolvable references and missing
//! examples in code blocks.

use super::{Check, fmt_cargo_cmd, run_cargo_outcome_with_env};
use crate::reporter::CheckOutcome;

const DOC_ARGS: &[&str] = &["--no-deps", "--workspace", "--all-features"];

pub struct DocCheck;

impl Check for DocCheck {
    fn label(&self) -> &'static str {
        "doc"
    }

    fn cmd(&self) -> String {
        format!(
            "RUSTDOCFLAGS='-D warnings' {}",
            fmt_cargo_cmd("doc", DOC_ARGS)
        )
    }

    fn run(&self) -> CheckOutcome {
        run_cargo_outcome_with_env("doc", DOC_ARGS, &[("RUSTDOCFLAGS", "-D warnings")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_doc() {
        assert_eq!(DocCheck.label(), "doc");
    }

    #[test]
    fn cmd_shows_rustdocflags_prefix() {
        let cmd = DocCheck.cmd();
        assert!(cmd.starts_with("RUSTDOCFLAGS='-D warnings' cargo doc "));
        assert!(cmd.contains("--no-deps"));
        assert!(cmd.contains("--workspace"));
    }
}
