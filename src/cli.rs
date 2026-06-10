// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use clap::{ColorChoice, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use serde::{Deserialize, Deserializer, de};

use crate::tooling::ColorMode;

/// Check identifier for `--skip`.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipOption {
    Check,
    Clippy,
    Test,
    DocTest,
    Fmt,
    Doc,
    Machete,
    Audit,
    License,
    Coverage,
}

impl SkipOption {
    /// Kebab-case identifier this variant accepts as `--skip <value>`.
    /// Single source of truth so hints in error messages cannot drift
    /// from what clap actually parses.
    #[must_use]
    pub const fn skip_flag(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Clippy => "clippy",
            Self::Test => "test",
            Self::DocTest => "doc-test",
            Self::Fmt => "fmt",
            Self::Doc => "doc",
            Self::Machete => "machete",
            Self::Audit => "audit",
            Self::License => "license",
            Self::Coverage => "coverage",
        }
    }

    fn from_flag(s: &str) -> Option<Self> {
        Self::value_variants()
            .iter()
            .find(|v| v.skip_flag() == s)
            .copied()
    }
}

/// Accept the same kebab-case identifiers `--skip <value>` does. Anchored
/// to [`SkipOption::skip_flag`] so CLI and Cargo.toml never diverge.
impl<'de> Deserialize<'de> for SkipOption {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Take ownership instead of borrowing: `serde_json::from_value`
        // walks a `Value` tree and cannot hand out borrowed strings.
        let raw = String::deserialize(deserializer)?;
        Self::from_flag(&raw).ok_or_else(|| {
            let known: Vec<&str> = Self::value_variants()
                .iter()
                .copied()
                .map(Self::skip_flag)
                .collect();
            de::Error::custom(format!(
                "unknown skip value `{raw}`; expected one of: {}",
                known.join(", "),
            ))
        })
    }
}

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Run every Rust quality gate in one command: compile, clippy, fmt, tests, doc, \
             doc-tests, machete, audit, plus opt-in license and coverage gates. One summary, \
             one exit code.",
    long_about = None,
    after_long_help = LONG_HELP_TAIL,
    // Cap help width even when stdout is not a TTY (pipes, CI logs). 100 is
    // wide enough for our longest line and lets clap wrap text that would
    // otherwise spill past `tput cols` in narrow terminals.
    max_term_width = 100,
    styles = clap_cargo::style::CLAP_STYLING
)]
pub struct Cli {
    /// Skip one or more checks (e.g. --skip clippy,fmt)
    //
    // `hide_possible_values` prevents clap from appending the auto-generated
    // `[possible values: ...]` line, which packed every variant onto a
    // single 170-char row that no terminal wrapped. The full list lives in
    // `long_help` (rendered by `--help`).
    #[arg(
        long,
        value_enum,
        value_name = "CHECK",
        value_delimiter = ',',
        hide_possible_values = true,
        long_help = "Skip one or more checks. Repeatable or comma-separated: \
                     `--skip clippy --skip fmt` or `--skip clippy,fmt`.\n\
                     \n\
                     Possible values: check, clippy, test, doc-test, fmt, doc, machete, \
                     audit, license, coverage."
    )]
    pub skip: Vec<SkipOption>,

    /// Show every command and the full output of all checks
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Auto-apply fmt, clippy --fix and machete --fix before the checks
    #[arg(
        long,
        long_help = "Auto-apply fmt, clippy --fix and machete --fix before the checks. \
                     Honours `--skip` (skipping clippy also skips its fix) and aborts \
                     the pipeline if any fix step fails."
    )]
    pub fix: bool,

    /// Run the opt-in coverage gate (thresholds default to 100%)
    #[arg(
        long,
        long_help = "Run the coverage gate even when `[*.metadata.lockpick.coverage]` is \
                     absent from Cargo.toml. Active thresholds default to 100% for every \
                     metric. Contradicts `--skip coverage` and `--skip test`, which is \
                     reported as a usage error rather than silently picking a winner."
    )]
    pub coverage: bool,

    /// Color policy. Honours `NO_COLOR` and TTY detection when `auto`
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        default_value_t = ColorChoice::Auto,
        long_help = "Coloured output policy. `auto` (the default) follows TTY detection \
                     and the `NO_COLOR` env var. `always`/`never` are explicit overrides \
                     that win over both signals."
    )]
    pub color: ColorChoice,

    /// Optional meta subcommand. `None` runs the default check pipeline.
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

/// Meta operations that bypass the check pipeline.
#[derive(Subcommand, Debug, Clone)]
pub enum Cmd {
    /// Emit a shell completion script for SHELL to stdout.
    ///
    /// Example (fish):
    ///   lockpick completions fish > ~/.config/fish/completions/lockpick.fish
    Completions {
        /// Target shell (bash, zsh, fish, powershell, elvish).
        shell: Shell,
    },
}

impl Cli {
    #[must_use]
    pub fn skips(&self, option: SkipOption) -> bool {
        self.skip.contains(&option)
    }

    /// Flatten the user's `--color` choice into the binary [`ColorMode`]
    /// every downstream consumer (subprocesses, `colored` override)
    /// expects. `Auto` defers to the TTY+`NO_COLOR` heuristic, explicit
    /// `always`/`never` wins outright.
    #[must_use]
    pub fn color_mode(&self, is_tty: bool) -> ColorMode {
        match self.color {
            ColorChoice::Always => ColorMode::Always,
            ColorChoice::Never => ColorMode::Never,
            ColorChoice::Auto => ColorMode::for_stdout(is_tty),
        }
    }

    /// Render the completion script for `shell` to `writer`. Sourced
    /// from the same `clap::Command` the parser uses, so the script can
    /// never describe a flag the binary does not accept.
    pub fn write_completions<W: std::io::Write>(shell: Shell, writer: &mut W) {
        let mut cmd = Self::command();
        let name = cmd.get_name().to_string();
        generate(shell, &mut cmd, name, writer);
    }

    /// Merge `config_skips` into `self.skip` in place. CLI order wins so
    /// error and diagnostic messages echo back what the user actually
    /// typed, with config entries as a stable, deduplicated tail.
    pub fn merge_config_skips(&mut self, config_skips: &[SkipOption]) {
        for s in config_skips {
            if !self.skip.contains(s) {
                self.skip.push(*s);
            }
        }
    }
}

/// Long-form `--help` tail: examples, environment variables, and the
/// `[*.metadata.lockpick]` schema reference. The Configuration section
/// must stay in sync with the keys serde accepts in
/// [`crate::config::Config`].
const LONG_HELP_TAIL: &str = "\
Examples:
  lockpick                          # run every check
  lockpick --skip clippy,fmt        # skip multiple checks (comma or repeated)
  lockpick --fix                    # auto-fix fmt, clippy and machete first
  lockpick --coverage               # also enforce the coverage gate (100% defaults)
  lockpick -v                       # show every cargo command and its full output
  lockpick --color=never            # force plain output (overrides NO_COLOR)

Environment:
  NO_COLOR    Non-empty value strips ANSI colors from lockpick and from every
              cargo subprocess it spawns. Honoured when `--color` is `auto`
              (the default). See <https://no-color.org>.

Configuration:
  Optional settings read from Cargo.toml under
  `[workspace.metadata.lockpick]` (preferred) or
  `[package.metadata.lockpick]`. Every field is optional. CLI `--skip`
  is additive on top of the `skip = [...]` array. The coverage gate is
  opt-in: it runs when the `coverage` table exists (even empty) or
  `--coverage` is passed.

      [workspace.metadata.lockpick]
      skip = [\"audit\", \"machete\"]
      license-header = \".github/license_header.rs\"
      license-header-globs = [\"src/**/*.rs\", \"tests/**/*.rs\"]

      # Presence of this table (even empty) enables the coverage gate.
      [workspace.metadata.lockpick.coverage]
      functions = 100   # every threshold defaults to 100
      lines     = 100
      regions   = 100
      # branches = 100  # opt-in, nightly-only (exit 4 on stable)
";

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn skip_flag_round_trips_every_variant() {
        for variant in SkipOption::value_variants() {
            assert_eq!(SkipOption::from_flag(variant.skip_flag()), Some(*variant));
        }
    }

    #[test]
    fn deserialize_accepts_kebab_case_identifiers() {
        let parsed: SkipOption = serde_json::from_value(serde_json::json!("doc-test")).unwrap();
        assert_eq!(parsed, SkipOption::DocTest);
    }

    #[test]
    fn deserialize_rejects_unknown_identifier_with_hint() {
        let err = serde_json::from_value::<SkipOption>(serde_json::json!("wat")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown skip value `wat`"), "got: {msg}");
        assert!(msg.contains("doc-test"), "hint must list variants: {msg}");
    }

    #[test]
    fn merge_config_skips_appends_without_duplicating() {
        let mut cli = Cli::parse_from(["lockpick", "--skip", "clippy"]);
        cli.merge_config_skips(&[SkipOption::Clippy, SkipOption::Fmt]);
        assert_eq!(cli.skip, vec![SkipOption::Clippy, SkipOption::Fmt]);
    }

    #[test]
    fn skips_reflects_parsed_values() {
        let cli = Cli::parse_from(["lockpick", "--skip", "audit,machete"]);
        assert!(cli.skips(SkipOption::Audit));
        assert!(cli.skips(SkipOption::Machete));
        assert!(!cli.skips(SkipOption::Coverage));
    }

    #[test]
    fn coverage_flag_defaults_off_and_parses_on() {
        assert!(!Cli::parse_from(["lockpick"]).coverage);
        assert!(Cli::parse_from(["lockpick", "--coverage"]).coverage);
    }

    #[test]
    fn explicit_color_choice_overrides_tty_state() {
        let always = Cli::parse_from(["lockpick", "--color", "always"]);
        let never = Cli::parse_from(["lockpick", "--color", "never"]);
        for is_tty in [true, false] {
            assert_eq!(always.color_mode(is_tty), ColorMode::Always);
            assert_eq!(never.color_mode(is_tty), ColorMode::Never);
        }
    }

    #[test]
    fn completions_render_a_nonempty_script() {
        let mut buf = Vec::new();
        Cli::write_completions(Shell::Bash, &mut buf);
        let script = String::from_utf8(buf).unwrap();
        assert!(script.contains("lockpick"), "got: {script}");
    }
}
