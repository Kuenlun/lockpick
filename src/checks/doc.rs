// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Documentation build with rustdoc warnings escalated to errors.
//! Catches broken intra-doc links, unresolvable references and missing
//! examples in code blocks.

use super::{Check, Runner, cargo_outcome_with_env, fmt_cargo_cmd};
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

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome_with_env(runner, "doc", DOC_ARGS, &[("RUSTDOCFLAGS", "-D warnings")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

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

    #[test]
    fn run_injects_rustdocflags_env() {
        let fake = FakeRunner::passing();
        assert!(DocCheck.run(&fake).passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "doc");
        assert_eq!(
            calls[0].envs,
            vec![("RUSTDOCFLAGS".to_string(), "-D warnings".to_string())]
        );
    }
}
