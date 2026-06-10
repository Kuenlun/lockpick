// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! `cargo doc` with `-D warnings` appended to any existing
//! `RUSTDOCFLAGS`, so broken intra-doc links and unresolvable
//! references fail the build without trampling user-supplied flags.

use super::{Check, Runner, cargo_outcome_with_env, chain, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;

const DOC_ARGS: &[&str] = &["--no-deps", "--workspace", "--all-features"];
const DENY_WARNINGS: &str = "-D warnings";

pub struct DocCheck;

impl Check for DocCheck {
    fn label(&self) -> &'static str {
        "doc"
    }

    fn cmd(&self) -> String {
        format!(
            "RUSTDOCFLAGS='{}' {}",
            rustdocflags(),
            fmt_cargo_cmd("doc", DOC_ARGS)
        )
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        let flags = rustdocflags();
        cargo_outcome_with_env(runner, "doc", DOC_ARGS, &[("RUSTDOCFLAGS", &flags)])
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::DOC)
    }
}

/// Read the current `RUSTDOCFLAGS` and append `-D warnings`.
fn rustdocflags() -> String {
    compose_rustdocflags(std::env::var("RUSTDOCFLAGS").ok())
}

/// Compose `RUSTDOCFLAGS` so the user's existing value survives.
///
/// `cargo doc` reads a single `RUSTDOCFLAGS` string, so naively overriding
/// it would erase flags the user needs (e.g. `--cfg docsrs` to gate
/// `#[doc(cfg(...))]` items). Append `-D warnings` instead.
fn compose_rustdocflags(existing: Option<String>) -> String {
    match existing {
        Some(flags) if !flags.trim().is_empty() => format!("{flags} {DENY_WARNINGS}"),
        _ => DENY_WARNINGS.to_string(),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn deny_warnings_is_appended_to_existing_flags() {
        assert_eq!(
            compose_rustdocflags(Some("--cfg docsrs".to_string())),
            "--cfg docsrs -D warnings"
        );
    }

    #[test]
    fn unset_or_blank_flags_become_deny_warnings_alone() {
        assert_eq!(compose_rustdocflags(None), "-D warnings");
        assert_eq!(compose_rustdocflags(Some("   ".to_string())), "-D warnings");
    }
}
