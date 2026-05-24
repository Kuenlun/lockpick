// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use clap::{
    ColorChoice, CommandFactory, Parser, Subcommand, ValueEnum,
    builder::styling::{AnsiColor, Effects, Styles},
};
use clap_complete::{Shell, generate};
use serde::{Deserialize, Deserializer, de};

use crate::tooling::ColorMode;

/// Check identifier for `--skip`.
#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
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
    /// from what clap actually parses (locked by a test below).
    #[must_use]
    pub const fn skip_flag(&self) -> &'static str {
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
            .cloned()
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
            let known: Vec<&str> = Self::value_variants().iter().map(Self::skip_flag).collect();
            de::Error::custom(format!(
                "unknown skip value `{raw}`; expected one of: {}",
                known.join(", "),
            ))
        })
    }
}

#[derive(Parser, Debug, Clone, Default)]
#[command(
    version,
    about = "Rust merge-check CLI. Runs compile, clippy, fmt, tests, doc, \
             doc-tests, machete, audit, license headers and 100% branch \
             coverage in a single invocation.",
    long_about = None,
    after_long_help = LONG_HELP_TAIL,
    // Cap help width even when stdout is not a TTY (pipes, CI logs). 100 is
    // wide enough for our longest line and lets clap wrap text that would
    // otherwise spill past `tput cols` in narrow terminals.
    max_term_width = 100,
    styles = cli_styles()
)]
pub struct Cli {
    /// Skip one or more checks (e.g. --skip clippy --skip fmt)
    //
    // `hide_possible_values` prevents clap from appending the auto-generated
    // `[possible values: ...]` line, which packed every variant onto a
    // single 170-char row that no terminal wrapped. The full list lives in
    // `long_help` (rendered by `--help`), kept in sync with the variants by
    // `long_help_lists_every_skip_value` further down.
    #[arg(
        long,
        value_enum,
        value_name = "CHECK",
        hide_possible_values = true,
        long_help = "Skip one or more checks. Repeatable, e.g. `--skip clippy --skip fmt`.\n\
                     \n\
                     Possible values: check, clippy, test, doc-test, fmt, doc, machete, \
                     audit, license, coverage."
    )]
    pub skip: Vec<SkipOption>,

    /// Show every command and the full output of all checks (CI mode)
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Coloured output policy. `auto` (the default) follows TTY detection
    /// and the `NO_COLOR` env var; `always`/`never` are explicit overrides
    /// that win over both signals.
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        default_value_t = ColorChoice::Auto,
    )]
    pub color: ColorChoice,

    /// Optional meta subcommand. `None` runs the default check pipeline.
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

/// Meta operations that bypass the check pipeline. Reserved for one-shot
/// utilities (completion scripts today, manpages tomorrow) whose output
/// is consumed by the shell or a packager, not the human running checks.
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
    pub fn skips(&self, option: &SkipOption) -> bool {
        self.skip.contains(option)
    }

    /// Flatten the user's `--color` choice into the binary [`ColorMode`]
    /// every downstream consumer (subprocesses, `colored` override)
    /// expects. `Auto` defers to the TTY+`NO_COLOR` heuristic; explicit
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

    /// Return a [`Cli`] whose `skip` list also includes every entry from
    /// `config_skips`, appended after the CLI-supplied skips and
    /// deduplicated. CLI order wins so error and diagnostic messages
    /// echo back what the user actually typed, with config entries as a
    /// stable tail.
    #[must_use]
    pub fn with_config_skips(&self, config_skips: &[SkipOption]) -> Self {
        let mut merged = self.clone();
        for s in config_skips {
            if !merged.skip.contains(s) {
                merged.skip.push(s.clone());
            }
        }
        merged
    }
}

/// Long-form `--help` tail. Three sibling sections, in narrative order:
///
/// * `Examples:` copy-paste invocations covering the common knobs.
/// * `Environment:` runtime levers the CLI surface cannot express
///   (only `NO_COLOR` today, see [`crate::tooling::ColorMode`]).
/// * `Configuration:` schema reference for `[*.metadata.lockpick]`,
///   mirroring the keys serde accepts in [`crate::config::Config`]. Add
///   or rename a field there and this block must follow suit.
///
/// Three tests guard the drift: `long_help_tail_covers_every_section`
/// (this file) pins the body, and the integration tests
/// `long_help_exposes_cargo_metadata_schema` and
/// `long_help_documents_examples_and_no_color_environment` pin what
/// clap actually renders.
const LONG_HELP_TAIL: &str = "\
Examples:
  lockpick                            # run every check
  lockpick --skip coverage            # skip the slow coverage gate
  lockpick --skip clippy --skip fmt   # skip multiple checks (repeatable)
  lockpick -v                         # CI mode: every cargo banner and section
  lockpick --color=never              # force plain output (overrides NO_COLOR)
  NO_COLOR=1 lockpick                 # plain ASCII output, no ANSI escapes

Environment:
  NO_COLOR    Set to any non-empty value to strip ANSI colors from lockpick's
              own output and from every cargo subprocess it spawns. Honoured
              when `--color` is `auto` (the default); explicit `--color
              always|never` wins. See <https://no-color.org>.

Configuration:
  Lockpick reads optional settings from your Cargo.toml under
  `[workspace.metadata.lockpick]` (preferred) or
  `[package.metadata.lockpick]`. Every field is optional.

      [workspace.metadata.lockpick]
      skip = [\"audit\", \"machete\"]                   # same identifiers as --skip
      license-header = \".github/license_header.rs\"
      license-header-globs = [\"src/**/*.rs\", \"tests/**/*.rs\"]

      [workspace.metadata.lockpick.coverage]
      functions = 100   # functions, lines and regions default to 100
      lines     = 100
      regions   = 100
      # branches = 100  # opt-in, nightly-only (exit 4 on stable)

  CLI `--skip` is additive on top of the `skip = [...]` array.
";

const fn cli_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .placeholder(AnsiColor::Blue.on_default())
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .valid(AnsiColor::Green.on_default().effects(Effects::BOLD))
        .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
}
