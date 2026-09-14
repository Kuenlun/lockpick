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

pub struct DocCheck {
    pub options: super::util::BuildOptions,
}

impl Check for DocCheck {
    fn label(&self) -> &'static str {
        "doc"
    }

    fn cmd(&self) -> String {
        format!(
            "{}={:?} {}",
            rustdocflags().0,
            rustdocflags().1,
            fmt_cargo_cmd("doc", &self.options.args(DOC_ARGS))
        )
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        let (key, flags) = rustdocflags();
        cargo_outcome_with_env(
            runner,
            "doc",
            &self.options.args(DOC_ARGS),
            &[(key, &flags)],
        )
    }

    fn chain_position(&self) -> Option<u8> {
        Some(chain::DOC)
    }
}

/// Read the current `RUSTDOCFLAGS` and append `-D warnings`.
fn rustdocflags() -> (&'static str, String) {
    compose_flags(
        std::env::var("CARGO_ENCODED_RUSTDOCFLAGS").ok(),
        std::env::var("RUSTDOCFLAGS").ok(),
    )
}

/// Cargo gives encoded flags precedence over the plain environment value.
fn compose_flags(encoded: Option<String>, plain: Option<String>) -> (&'static str, String) {
    encoded.map_or_else(
        || ("RUSTDOCFLAGS", compose_rustdocflags(plain)),
        |mut flags| {
            if !flags.is_empty() {
                flags.push('\u{1f}');
            }
            flags.push_str("-D\u{1f}warnings");
            ("CARGO_ENCODED_RUSTDOCFLAGS", flags)
        },
    )
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
    #[test]
    fn encoded_flags_keep_precedence_and_argument_boundaries() {
        assert_eq!(
            compose_flags(Some("--cfg\u{1f}custom".into()), Some("ignored".into())),
            (
                "CARGO_ENCODED_RUSTDOCFLAGS",
                "--cfg\u{1f}custom\u{1f}-D\u{1f}warnings".into()
            )
        );
        assert_eq!(
            compose_flags(Some(String::new()), None),
            ("CARGO_ENCODED_RUSTDOCFLAGS", "-D\u{1f}warnings".into())
        );
        assert_eq!(
            compose_flags(None, None),
            ("RUSTDOCFLAGS", "-D warnings".into())
        );
    }
}
