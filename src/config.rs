// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Lockpick configuration loaded from `[workspace.metadata.lockpick]`
//! or `[package.metadata.lockpick]` in `Cargo.toml`. Read transparently
//! via `cargo metadata --format-version 1 --no-deps`.

use std::path::PathBuf;
use std::process::Stdio;

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
struct CargoMetadata {
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
        let Some(metadata) = run_cargo_metadata() else {
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
    let output = cargo_command()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn extract_lockpick(metadata: &CargoMetadata) -> Option<Value> {
    if let Value::Object(map) = &metadata.workspace_metadata
        && let Some(v) = map.get("lockpick")
    {
        return Some(v.clone());
    }
    if let [package] = metadata.packages.as_slice()
        && let Value::Object(map) = &package.metadata
        && let Some(v) = map.get("lockpick")
    {
        return Some(v.clone());
    }
    None
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
        // With multiple packages and no workspace metadata, we don't aggregate.
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
        // Omitted fields keep their default of 100.
        assert_eq!(cfg.coverage.regions, 100);
        assert_eq!(cfg.coverage.branches, 100);
    }
}
