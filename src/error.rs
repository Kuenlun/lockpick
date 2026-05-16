// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::fmt::Write;

use colored::Colorize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockpickError {
    #[error("{0} check(s) failed")]
    ChecksFailed(usize),

    /// One or more required cargo subcommands are absent from `PATH`.
    /// The Display impl bundles every entry into a single message with
    /// a unified `cargo install …` line so the user only hits this
    /// error once per pipeline run.
    #[error("{}", render_missing(.0))]
    MissingTools(Vec<MissingTool>),

    /// Every check was disabled via `--skip`, leaving the pipeline
    /// empty. Reported as a misconfiguration rather than success so a
    /// merge gate that ran nothing never reads as green in CI.
    #[error("all checks skipped; nothing to verify")]
    NoChecksToRun,

    /// The user configured `coverage.branches` but the active toolchain
    /// is stable. Branch coverage relies on `-Z coverage-options=branch`,
    /// which only nightly accepts, so refusing up front beats handing
    /// back a raw `rustc` error mid-pipeline.
    #[error("{}", render_branches_nightly())]
    BranchesRequireNightly,
}

/// One absent cargo subcommand row used by [`LockpickError::MissingTools`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingTool {
    /// Binary name on `PATH`, doubles as the `cargo install` argument.
    pub binary: &'static str,
    /// `--skip` value that disables the dependent check.
    pub skip_flag: &'static str,
}

/// Render the bundled install-or-skip hint. Caller ensures `missing`
/// is non-empty.
fn render_missing(missing: &[MissingTool]) -> String {
    debug_assert!(
        !missing.is_empty(),
        "render_missing requires at least one entry"
    );
    let n = missing.len();
    let (noun, verb) = if n == 1 {
        ("tool", "is")
    } else {
        ("tools", "are")
    };
    let width = missing.iter().map(|m| m.binary.len()).max().unwrap_or(0);

    let mut out = String::new();
    let _ = writeln!(&mut out, "{n} required {noun} {verb} missing:");
    out.push('\n');
    for m in missing {
        let bin = format!("{:<width$}", m.binary, width = width);
        let _ = writeln!(
            &mut out,
            "  {bullet} {bin}  (needed for: {check})",
            bullet = "•".dimmed(),
            bin = bin.yellow().bold(),
            check = m.skip_flag.cyan(),
        );
    }

    let install_cmd = format!(
        "cargo install {}",
        missing
            .iter()
            .map(|m| m.binary)
            .collect::<Vec<_>>()
            .join(" ")
    );
    let skip_cmd = format!(
        "lockpick {}",
        missing
            .iter()
            .map(|m| format!("--skip {}", m.skip_flag))
            .collect::<Vec<_>>()
            .join(" ")
    );

    out.push('\n');
    let _ = writeln!(&mut out, "{}", "Install:".bold());
    let _ = writeln!(&mut out, "  {}", install_cmd.cyan().bold());
    out.push('\n');
    let _ = writeln!(&mut out, "{}", "Or skip:".bold());
    let _ = write!(&mut out, "  {}", skip_cmd.cyan());

    out
}

/// Render the actionable hint for [`LockpickError::BranchesRequireNightly`].
///
/// Mirrors the layout of [`render_missing`] so users see a familiar
/// "what is wrong / how to fix" structure across error variants.
fn render_branches_nightly() -> String {
    let mut out = String::new();
    let _ = writeln!(
        &mut out,
        "{key} requires nightly Rust",
        key = "coverage.branches".yellow().bold(),
    );
    out.push('\n');
    let _ = writeln!(
        &mut out,
        "Branch coverage uses `-Z coverage-options=branch`, which only nightly accepts."
    );
    out.push('\n');
    let _ = writeln!(&mut out, "{}", "Either:".bold());
    let _ = writeln!(
        &mut out,
        "  {bullet} remove {key} from [*.metadata.lockpick.coverage]",
        bullet = "•".dimmed(),
        key = "branches".cyan(),
    );
    let _ = write!(
        &mut out,
        "  {bullet} install nightly: {cmd}",
        bullet = "•".dimmed(),
        cmd = "rustup toolchain install nightly".cyan().bold(),
    );

    out
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    const LLVM_COV: MissingTool = MissingTool {
        binary: "cargo-llvm-cov",
        skip_flag: "coverage",
    };
    const MACHETE: MissingTool = MissingTool {
        binary: "cargo-machete",
        skip_flag: "machete",
    };
    const AUDIT: MissingTool = MissingTool {
        binary: "cargo-audit",
        skip_flag: "audit",
    };

    /// Strip ANSI SGR escapes (`ESC [ params LETTER`) so assertions stay
    /// independent of `colored`'s tty auto-detection. Sufficient because
    /// `colored` only emits SGR, never OSC or other CSI shapes.
    fn plain(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for skipped in chars.by_ref() {
                    if skipped.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn checks_failed_display_uses_pluralized_noun() {
        assert_eq!(
            LockpickError::ChecksFailed(3).to_string(),
            "3 check(s) failed"
        );
    }

    #[test]
    fn render_missing_single_tool_uses_singular_header() {
        let msg = plain(&render_missing(&[LLVM_COV]));
        assert!(
            msg.starts_with("1 required tool is missing:\n"),
            "got: {msg}"
        );
        assert!(msg.contains("• cargo-llvm-cov"), "got: {msg}");
        assert!(msg.contains("(needed for: coverage)"), "got: {msg}");
        assert!(
            msg.contains("\nInstall:\n  cargo install cargo-llvm-cov\n"),
            "got: {msg}"
        );
        assert!(
            msg.contains("\nOr skip:\n  lockpick --skip coverage"),
            "got: {msg}"
        );
    }

    #[test]
    fn render_missing_multiple_tools_bundles_install_and_skip_into_one_line_each() {
        let msg = plain(&render_missing(&[LLVM_COV, MACHETE, AUDIT]));
        assert!(
            msg.starts_with("3 required tools are missing:\n"),
            "got: {msg}"
        );
        // Every tool appears as its own bullet row.
        for tool in ["cargo-llvm-cov", "cargo-machete", "cargo-audit"] {
            assert!(
                msg.contains(&format!("• {tool}")),
                "missing bullet for {tool}:\n{msg}"
            );
        }
        // Single combined install command.
        assert!(
            msg.contains("cargo install cargo-llvm-cov cargo-machete cargo-audit"),
            "expected combined install line, got: {msg}"
        );
        // Single combined skip command.
        assert!(
            msg.contains("lockpick --skip coverage --skip machete --skip audit"),
            "expected combined skip line, got: {msg}"
        );
    }

    #[test]
    fn render_missing_aligns_bullet_rows_to_the_longest_binary_name() {
        let msg = plain(&render_missing(&[LLVM_COV, MACHETE, AUDIT]));
        let suffix = "  (needed for:";
        // `cargo-llvm-cov` is the widest binary name; rows for shorter
        // names must be padded so the `(needed for:` column lines up.
        let columns: Vec<usize> = msg.lines().filter_map(|l| l.find(suffix)).collect();
        assert!(columns.len() >= 2, "expected >=2 bullet rows, got: {msg}");
        assert!(
            columns.windows(2).all(|w| w[0] == w[1]),
            "bullet rows are not column-aligned: {columns:?}\n{msg}",
        );
    }

    #[test]
    fn missing_tools_display_renders_through_the_error_variant() {
        let err = LockpickError::MissingTools(vec![LLVM_COV]);
        let s = plain(&err.to_string());
        assert!(s.contains("cargo install cargo-llvm-cov"), "got: {s}");
    }

    #[test]
    fn no_checks_to_run_display_is_a_short_actionable_message() {
        assert_eq!(
            LockpickError::NoChecksToRun.to_string(),
            "all checks skipped; nothing to verify"
        );
    }

    #[test]
    fn render_branches_nightly_names_the_offending_key_and_offers_two_remedies() {
        let msg = plain(&render_branches_nightly());
        assert!(
            msg.contains("coverage.branches"),
            "expected the offending key in the header, got: {msg}"
        );
        assert!(
            msg.contains("requires nightly Rust"),
            "expected the nightly requirement to be named, got: {msg}"
        );
        // Both escape hatches must be present so the user knows there
        // are two valid routes out of the failure.
        assert!(
            msg.contains("remove") && msg.contains("branches"),
            "expected the `remove branches` remedy, got: {msg}"
        );
        assert!(
            msg.contains("rustup toolchain install nightly"),
            "expected the `install nightly` remedy, got: {msg}"
        );
    }

    #[test]
    fn branches_require_nightly_display_renders_through_the_error_variant() {
        let err = LockpickError::BranchesRequireNightly;
        let s = plain(&err.to_string());
        assert!(s.contains("coverage.branches"), "got: {s}");
        assert!(s.contains("nightly"), "got: {s}");
    }
}
