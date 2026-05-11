// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Lockpick configuration loaded from `[workspace.metadata.lockpick]`
//! or `[package.metadata.lockpick]` in `Cargo.toml`. Read transparently
//! via `cargo metadata --format-version 1 --no-deps`.

use std::path::PathBuf;
use std::process::{Output, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::tooling::cargo_command;

/// Per-metric coverage thresholds. Defaults to 100% on every metric.
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(default)]
pub struct CoverageConfig {
    pub functions: u8,
    pub lines: u8,
    pub regions: u8,
    pub branches: u8,
}

impl Default for CoverageConfig {
    fn default() -> Self {
        Self {
            functions: 100,
            lines: 100,
            regions: 100,
            branches: 100,
        }
    }
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub license_header: Option<PathBuf>,
    pub license_header_globs: Option<Vec<String>>,
    pub coverage: CoverageConfig,
}

#[derive(Deserialize, Default)]
pub struct CargoMetadata {
    #[serde(default)]
    workspace_metadata: Value,
    #[serde(default)]
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize, Default)]
struct CargoPackage {
    #[serde(default)]
    metadata: Value,
}

impl Config {
    /// Load lockpick configuration via `cargo metadata`. Falls back to
    /// defaults when no `[*.metadata.lockpick]` section is present, when
    /// the section fails to deserialize, or when `cargo metadata` itself
    /// cannot run (e.g. outside a cargo project).
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(run_cargo_metadata())
    }

    /// Pure variant of [`Self::load`] that takes the already-fetched
    /// metadata so unit tests can drive every branch deterministically.
    pub fn load_from(metadata: Option<CargoMetadata>) -> Self {
        let Some(metadata) = metadata else {
            return Self::default();
        };
        let Some(section) = extract_lockpick(&metadata) else {
            return Self::default();
        };
        serde_json::from_value(section).unwrap_or_else(|e| {
            eprintln!("warning: invalid [*.metadata.lockpick] section: {e} — using defaults");
            Self::default()
        })
    }
}

fn run_cargo_metadata() -> Option<CargoMetadata> {
    parse_cargo_metadata(
        cargo_command()
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .stderr(Stdio::null())
            .output(),
    )
}

/// Pure helper: lower a `cargo metadata` spawn result into the parsed
/// [`CargoMetadata`]. Returns `None` for every error path the production
/// code already silently tolerates (spawn failed, cargo exited non-zero,
/// stdout was not valid metadata JSON).
fn parse_cargo_metadata(result: std::io::Result<Output>) -> Option<CargoMetadata> {
    let output = result.ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn extract_lockpick(metadata: &CargoMetadata) -> Option<Value> {
    fn lockpick_in(value: &Value) -> Option<Value> {
        value.as_object().and_then(|m| m.get("lockpick")).cloned()
    }
    lockpick_in(&metadata.workspace_metadata).or_else(|| {
        let [package] = metadata.packages.as_slice() else {
            return None;
        };
        lockpick_in(&package.metadata)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta_with(workspace: Value, packages: Vec<Value>) -> CargoMetadata {
        CargoMetadata {
            workspace_metadata: workspace,
            packages: packages
                .into_iter()
                .map(|metadata| CargoPackage { metadata })
                .collect(),
        }
    }

    #[test]
    fn coverage_config_defaults_to_100_on_every_metric() {
        let c = CoverageConfig::default();
        assert_eq!(c.functions, 100);
        assert_eq!(c.lines, 100);
        assert_eq!(c.regions, 100);
        assert_eq!(c.branches, 100);
    }

    #[test]
    fn config_default_has_no_license_header() {
        let c = Config::default();
        assert!(c.license_header.is_none());
        assert!(c.license_header_globs.is_none());
        assert_eq!(c.coverage.functions, 100);
    }

    #[test]
    fn extract_lockpick_prefers_workspace_metadata() {
        let meta = meta_with(
            json!({ "lockpick": { "license-header": "ws.txt" } }),
            vec![json!({ "lockpick": { "license-header": "pkg.txt" } })],
        );
        let v = extract_lockpick(&meta).expect("found");
        assert_eq!(v["license-header"], "ws.txt");
    }

    #[test]
    fn extract_lockpick_falls_back_to_single_package_metadata() {
        let meta = meta_with(
            Value::Null,
            vec![json!({ "lockpick": { "license-header": "pkg.txt" } })],
        );
        let v = extract_lockpick(&meta).expect("found");
        assert_eq!(v["license-header"], "pkg.txt");
    }

    #[test]
    fn extract_lockpick_skips_package_metadata_when_multi_crate_workspace() {
        let meta = meta_with(
            Value::Null,
            vec![
                json!({ "lockpick": { "license-header": "a.txt" } }),
                json!({ "lockpick": { "license-header": "b.txt" } }),
            ],
        );
        assert!(extract_lockpick(&meta).is_none());
    }

    #[test]
    fn extract_lockpick_returns_none_when_section_is_absent() {
        let meta = meta_with(json!({ "other": {} }), vec![json!({ "other": {} })]);
        assert!(extract_lockpick(&meta).is_none());
    }

    #[test]
    fn extract_lockpick_returns_none_when_workspace_metadata_lacks_lockpick_key() {
        let meta = meta_with(json!({ "other": "x" }), vec![]);
        assert!(extract_lockpick(&meta).is_none());
    }

    #[test]
    fn extract_lockpick_returns_none_when_single_package_metadata_is_not_object() {
        let meta = meta_with(Value::Null, vec![json!("a string, not an object")]);
        assert!(extract_lockpick(&meta).is_none());
    }

    #[test]
    fn config_deserializes_kebab_case_fields() {
        let v = json!({
            "license-header": "header.txt",
            "license-header-globs": ["src/**/*.rs"],
            "coverage": { "functions": 90, "lines": 95 }
        });
        let cfg: Config = serde_json::from_value(v).unwrap();
        assert_eq!(
            cfg.license_header.as_deref(),
            Some(std::path::Path::new("header.txt"))
        );
        assert_eq!(
            cfg.license_header_globs.as_deref(),
            Some(&["src/**/*.rs".to_string()][..])
        );
        assert_eq!(cfg.coverage.functions, 90);
        assert_eq!(cfg.coverage.lines, 95);
        assert_eq!(cfg.coverage.regions, 100);
        assert_eq!(cfg.coverage.branches, 100);
    }

    #[test]
    fn load_from_none_returns_defaults() {
        let cfg = Config::load_from(None);
        assert!(cfg.license_header.is_none());
    }

    #[test]
    fn load_from_metadata_without_lockpick_section_returns_defaults() {
        let cfg = Config::load_from(Some(meta_with(json!({ "other": {} }), vec![])));
        assert!(cfg.license_header.is_none());
    }

    #[test]
    fn load_from_metadata_with_lockpick_section_applies_overrides() {
        let cfg = Config::load_from(Some(meta_with(
            json!({ "lockpick": { "license-header": "hdr.txt" } }),
            vec![],
        )));
        assert_eq!(
            cfg.license_header.as_deref(),
            Some(std::path::Path::new("hdr.txt"))
        );
    }

    #[test]
    fn load_from_falls_back_to_defaults_on_invalid_section_and_warns() {
        // `coverage` must deserialize to CoverageConfig; passing a string
        // forces a deserialization error and exercises the warning branch.
        let cfg = Config::load_from(Some(meta_with(
            json!({ "lockpick": { "coverage": "not a number" } }),
            vec![],
        )));
        assert!(cfg.license_header.is_none());
        assert_eq!(cfg.coverage.functions, 100);
    }

    #[test]
    fn load_smoke_test_against_real_cargo_metadata() {
        // Real cargo metadata works in lockpick's own repo; this exercises
        // the production `Config::load` wrapper end-to-end.
        let _ = Config::load();
    }

    #[test]
    fn run_cargo_metadata_returns_some_when_invoked_inside_cargo_project() {
        let meta = run_cargo_metadata();
        assert!(meta.is_some(), "expected cargo metadata to succeed");
    }

    #[test]
    fn parse_cargo_metadata_returns_none_when_spawn_failed() {
        let result: std::io::Result<std::process::Output> = Err(std::io::Error::other("ENOENT"));
        assert!(parse_cargo_metadata(result).is_none());
    }

    #[test]
    fn parse_cargo_metadata_returns_none_when_cargo_exited_non_zero() {
        let out = std::process::Command::new("cargo")
            .arg("definitely-not-a-real-subcommand-config")
            .output()
            .expect("cargo runs");
        assert!(!out.status.success());
        assert!(parse_cargo_metadata(Ok(out)).is_none());
    }

    #[test]
    fn parse_cargo_metadata_returns_none_when_stdout_is_not_metadata_json() {
        let out = std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .expect("cargo runs");
        assert!(out.status.success());
        assert!(parse_cargo_metadata(Ok(out)).is_none());
    }

    #[test]
    fn parse_cargo_metadata_returns_some_for_real_cargo_metadata_output() {
        let out = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .output()
            .expect("cargo runs");
        assert!(parse_cargo_metadata(Ok(out)).is_some());
    }
}
