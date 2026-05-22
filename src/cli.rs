// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use clap::{
    ColorChoice, Parser, ValueEnum,
    builder::styling::{AnsiColor, Effects, Styles},
};
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Anchor `skip_flag` to clap's derived name for every variant, so a
    /// rename or kebab-case tweak fails here instead of silently shipping
    /// a hint pointing at a flag clap no longer accepts.
    #[test]
    fn skip_flag_matches_clap_value_enum_name_for_every_variant() {
        for variant in SkipOption::value_variants() {
            assert_eq!(
                variant.skip_flag(),
                variant.to_possible_value().unwrap().get_name(),
                "skip_flag drift for {variant:?}",
            );
        }
    }

    /// Deserialization must accept exactly the kebab-case identifiers
    /// `--skip` does, so users can copy a CLI invocation straight into
    /// `skip = [...]` without translation.
    #[test]
    fn skip_option_deserializes_every_value_enum_name() {
        for variant in SkipOption::value_variants() {
            let json = format!("\"{}\"", variant.skip_flag());
            let parsed: SkipOption = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("failed to deserialize {json}: {e}"));
            assert_eq!(&parsed, variant);
        }
    }

    #[test]
    fn skip_option_rejects_unknown_value_with_known_list_in_error() {
        let err = serde_json::from_str::<SkipOption>("\"klippy\"").expect_err("typo must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("klippy"),
            "error should echo the bad value: {msg}"
        );
        assert!(
            msg.contains("clippy"),
            "error should list known values: {msg}"
        );
    }

    /// Pin the `?` propagation in [`SkipOption::deserialize`]: feeding a
    /// non-string node must short-circuit on `String::deserialize` before
    /// the kebab-case lookup ever runs.
    #[test]
    fn skip_option_rejects_non_string_input_via_string_deserialize() {
        let err = serde_json::from_str::<SkipOption>("42").expect_err("number must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("string"),
            "expected a type-mismatch error mentioning `string`, got: {msg}"
        );
    }

    fn cli_with_skips(skips: &[SkipOption]) -> Cli {
        Cli {
            skip: skips.to_vec(),
            ..Cli::default()
        }
    }

    #[test]
    fn with_config_skips_appends_new_entries_preserving_cli_order() {
        let cli = cli_with_skips(&[SkipOption::Fmt, SkipOption::Clippy]);
        let merged = cli.with_config_skips(&[SkipOption::Audit, SkipOption::Machete]);
        assert_eq!(
            merged.skip,
            vec![
                SkipOption::Fmt,
                SkipOption::Clippy,
                SkipOption::Audit,
                SkipOption::Machete,
            ],
        );
    }

    #[test]
    fn with_config_skips_deduplicates_against_cli_entries() {
        let cli = cli_with_skips(&[SkipOption::Audit]);
        let merged = cli.with_config_skips(&[SkipOption::Audit, SkipOption::Machete]);
        assert_eq!(merged.skip, vec![SkipOption::Audit, SkipOption::Machete]);
    }

    #[test]
    fn with_config_skips_is_a_no_op_for_empty_config_list() {
        let cli = cli_with_skips(&[SkipOption::Fmt]);
        let merged = cli.with_config_skips(&[]);
        assert_eq!(merged.skip, vec![SkipOption::Fmt]);
    }

    /// Pin the three top-level sections of the long-help tail, the
    /// schema example, and the `NO_COLOR` mention. Renaming or dropping
    /// any of them surfaces a fix-it failure right next to the constant
    /// instead of waiting on the integration tests.
    #[test]
    fn long_help_tail_covers_every_section() {
        for header in ["Examples:", "Environment:", "Configuration:"] {
            assert!(
                LONG_HELP_TAIL.contains(header),
                "long-help tail must include `{header}`, got:\n{LONG_HELP_TAIL}",
            );
        }
        assert!(
            LONG_HELP_TAIL.contains("NO_COLOR"),
            "Environment block must document `NO_COLOR`, got:\n{LONG_HELP_TAIL}",
        );
        assert!(
            LONG_HELP_TAIL.contains("skip = ["),
            "schema block must show the `skip` array form",
        );
        for section in [
            "[workspace.metadata.lockpick]",
            "[workspace.metadata.lockpick.coverage]",
            "license-header",
            "license-header-globs",
            "branches",
        ] {
            assert!(
                LONG_HELP_TAIL.contains(section),
                "schema block must mention `{section}`, got:\n{LONG_HELP_TAIL}",
            );
        }
    }

    /// Pin every branch of [`Cli::color_mode`]: `Always`/`Never` must
    /// override the TTY heuristic outright, `Auto` must defer to it.
    /// The pipe path of `Auto` is the only one independent of `NO_COLOR`,
    /// so it stays race-free across the test runner.
    #[test]
    fn color_mode_resolves_each_choice_into_the_expected_color_mode() {
        let always = Cli {
            color: ColorChoice::Always,
            ..Cli::default()
        };
        assert_eq!(always.color_mode(false), ColorMode::Always);

        let never = Cli {
            color: ColorChoice::Never,
            ..Cli::default()
        };
        assert_eq!(never.color_mode(true), ColorMode::Never);

        let auto = Cli::default();
        assert_eq!(auto.color_mode(false), ColorMode::Never);
    }

    /// Possible values are hidden from clap's auto-generated suffix; the
    /// human-readable list lives in the `--skip` `long_help` instead.
    /// Pin every variant so a rename of [`SkipOption::skip_flag`] fails
    /// here rather than silently shipping a stale help string.
    #[test]
    fn long_help_lists_every_skip_value() {
        let command = Cli::command();
        let skip_arg = command
            .get_arguments()
            .find(|a| a.get_id() == "skip")
            .expect("--skip arg must exist");
        let long_help = skip_arg
            .get_long_help()
            .expect("--skip must define long_help so users see the value list")
            .to_string();
        assert!(
            skip_arg.is_hide_possible_values_set(),
            "clap's auto-listed possible-values block must stay hidden so it cannot reintroduce the 170-char line",
        );
        for variant in SkipOption::value_variants() {
            assert!(
                long_help.contains(variant.skip_flag()),
                "--skip long_help must list `{}`, got:\n{long_help}",
                variant.skip_flag(),
            );
        }
    }
}
