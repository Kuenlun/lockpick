// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::fmt::Write;

use colored::Colorize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockpickError {
    #[error("{0} check(s) failed")]
    ChecksFailed(usize),

    /// One or more required cargo subcommands are absent from `PATH`.
    /// Bundled into a single message with a unified `cargo install …`
    /// line so the user only hits this error once per run.
    #[error("{}", render_missing(.0))]
    MissingTools(Vec<MissingTool>),

    /// Every check was skipped, leaving the pipeline empty. Reported
    /// as a misconfiguration so a merge gate that ran nothing never
    /// reads as green.
    #[error("all checks skipped; nothing to verify")]
    NoChecksToRun,

    /// `coverage.branches` configured on a stable toolchain. Refusing
    /// up front beats handing back a raw `rustc` error mid-pipeline.
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
/// Mirrors the layout of [`render_missing`] for a familiar
/// "what is wrong / how to fix" shape.
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
