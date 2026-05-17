// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use clap::{
    Parser, ValueEnum,
    builder::styling::{AnsiColor, Effects, Styles},
};
use serde::{Deserialize, Deserializer, de};

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

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Rust merge-check CLI. Runs compile, clippy, fmt, tests, doc, \
             doc-tests, machete, audit, license headers and 100% branch \
             coverage in a single invocation.",
    long_about = None,
    after_long_help = CONFIGURATION_HELP,
    styles = cli_styles()
)]
pub struct Cli {
    /// Skip one or more checks (e.g. --skip clippy --skip fmt)
    #[arg(long, value_enum)]
    pub skip: Vec<SkipOption>,

    /// Show every command and the full output of all checks (CI mode)
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
}

impl Cli {
    #[must_use]
    pub fn skips(&self, option: &SkipOption) -> bool {
        self.skip.contains(option)
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

/// `--help` schema reference for `[*.metadata.lockpick]`. Mirrors the
/// keys serde accepts in [`crate::config::Config`]. Add or rename a
/// field there and this block must follow suit. Two tests guard the
/// drift: `configuration_help_mentions_skip_array_and_every_known_section`
/// pins the body, and the integration test
/// `long_help_exposes_cargo_metadata_schema` pins the rendered output.
const CONFIGURATION_HELP: &str = "\
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
            verbose: false,
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

    /// Anchor every variant of [`SkipOption`] in the `--help` schema
    /// block so adding a check forces the configuration example to be
    /// updated in the same diff.
    #[test]
    fn configuration_help_mentions_skip_array_and_every_known_section() {
        assert!(
            CONFIGURATION_HELP.contains("skip = ["),
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
                CONFIGURATION_HELP.contains(section),
                "schema block must mention `{section}`, got:\n{CONFIGURATION_HELP}",
            );
        }
    }
}
