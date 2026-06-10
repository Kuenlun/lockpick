// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Lockpick configuration loaded from `[workspace.metadata.lockpick]`
//! (preferred) or `[package.metadata.lockpick]` via `cargo metadata`.

use std::path::PathBuf;
use std::process::Stdio;

use serde::Deserialize;
use serde_json::Value;

use crate::cli::SkipOption;
use crate::tooling::cargo_command;

/// Per-metric coverage thresholds.
///
/// The coverage gate itself is opt-in (see [`Config::coverage`]). Once
/// active, `functions`, `lines` and `regions` default to 100%.
/// `branches` is optional because branch coverage requires nightly:
/// unset defaults to 100% on nightly and is silently dropped on
/// stable. An explicit value causes lockpick to refuse to run on stable.
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(default, deny_unknown_fields)]
pub struct CoverageConfig {
    pub functions: u8,
    pub lines: u8,
    pub regions: u8,
    pub branches: Option<u8>,
}

impl Default for CoverageConfig {
    fn default() -> Self {
        Self {
            functions: 100,
            lines: 100,
            regions: 100,
            branches: None,
        }
    }
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub license_header: Option<PathBuf>,
    pub license_header_globs: Option<Vec<String>>,
    /// Opt-in coverage gate. `Some` whenever the
    /// `[*.metadata.lockpick.coverage]` table exists, even empty, with
    /// per-metric thresholds defaulting to 100%. `None` keeps coverage
    /// off unless the CLI passes `--coverage`.
    pub coverage: Option<CoverageConfig>,
    /// Project-wide skip list. Same kebab-case identifiers `--skip`
    /// accepts on the CLI, merged with (not replaced by) any CLI flags.
    pub skip: Vec<SkipOption>,
}

/// Lockpick config and workspace facts derived from a single
/// `cargo metadata` invocation.
#[derive(Debug, Clone, Default)]
pub struct LockpickMetadata {
    pub config: Config,
    pub has_lib_target: bool,
    /// Absolute path of the enclosing workspace. `None` when the probe
    /// failed (no Cargo.toml in scope, malformed JSON, ...).
    pub workspace_root: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
struct CargoMetadata {
    // Cargo emits this key as plain `metadata`, not `workspace_metadata`.
    #[serde(default, rename = "metadata")]
    workspace_metadata: Value,
    #[serde(default)]
    workspace_root: Option<PathBuf>,
    #[serde(default)]
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize, Default)]
struct CargoPackage {
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    targets: Vec<CargoTarget>,
}

#[derive(Deserialize, Default)]
struct CargoTarget {
    #[serde(default)]
    kind: Vec<String>,
}

/// Crate types Cargo treats as library targets, all of which can carry
/// doctests. A plain `kind == "lib"` check misses `cdylib`,
/// `proc-macro`, etc., and silently skips the doc-test gate.
const LIB_KINDS: &[&str] = &["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"];

impl LockpickMetadata {
    /// Probe `cargo metadata` and fall back to defaults on any failure.
    #[must_use]
    pub fn load() -> Self {
        let Some(metadata) = run_cargo_metadata() else {
            return Self::default();
        };
        let has_lib_target = metadata
            .packages
            .iter()
            .flat_map(|p| &p.targets)
            .any(|t| t.kind.iter().any(|k| LIB_KINDS.contains(&k.as_str())));
        let config = extract_lockpick(&metadata).map_or_else(Config::default, deserialize_or_warn);
        Self {
            config,
            has_lib_target,
            workspace_root: metadata.workspace_root,
        }
    }
}

fn deserialize_or_warn(section: Value) -> Config {
    serde_json::from_value(section).unwrap_or_else(|e| {
        eprintln!("warning: invalid [*.metadata.lockpick] section: {e}, using defaults");
        Config::default()
    })
}

/// Spawn `cargo metadata` and parse its JSON. Returns `None` on any
/// failure (spawn error, non-zero exit, or malformed JSON).
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

/// Locate `[*.metadata.lockpick]` in priority order:
///
/// 1. `[workspace.metadata.lockpick]`.
/// 2. `[package.metadata.lockpick]` of a single-package workspace.
///
/// Multi-package workspaces that set `[package.metadata.lockpick]` without
/// the workspace-scoped section get a warning: there is no safe winner to
/// pick workspace-wide, so the configuration is dropped.
fn extract_lockpick(metadata: &CargoMetadata) -> Option<Value> {
    fn lockpick_in(value: &Value) -> Option<Value> {
        value.as_object().and_then(|m| m.get("lockpick")).cloned()
    }
    if let Some(ws) = lockpick_in(&metadata.workspace_metadata) {
        return Some(ws);
    }
    if let [package] = metadata.packages.as_slice() {
        return lockpick_in(&package.metadata);
    }
    let stray = metadata
        .packages
        .iter()
        .filter(|p| lockpick_in(&p.metadata).is_some())
        .count();
    if stray > 0 {
        eprintln!(
            "warning: found `[package.metadata.lockpick]` in {stray} package(s) of a multi-crate workspace. Use `[workspace.metadata.lockpick]` to apply it workspace-wide"
        );
    }
    None
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cli::SkipOption;

    fn metadata_from(value: serde_json::Value) -> CargoMetadata {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn coverage_table_is_off_by_default_and_on_when_present() {
        let absent: Config = serde_json::from_value(json!({})).unwrap();
        assert!(absent.coverage.is_none());

        let present: Config = serde_json::from_value(json!({ "coverage": {} })).unwrap();
        let thresholds = present.coverage.unwrap();
        assert_eq!(thresholds.functions, 100);
        assert_eq!(thresholds.lines, 100);
        assert_eq!(thresholds.regions, 100);
        assert!(thresholds.branches.is_none());
    }

    #[test]
    fn config_accepts_kebab_case_fields_and_skip_list() {
        let config: Config = serde_json::from_value(json!({
            "license-header": "hdr.txt",
            "skip": ["audit", "machete"],
            "coverage": { "lines": 90 },
        }))
        .unwrap();
        assert_eq!(config.license_header.unwrap(), PathBuf::from("hdr.txt"));
        assert_eq!(config.skip, vec![SkipOption::Audit, SkipOption::Machete]);
        assert_eq!(config.coverage.unwrap().lines, 90);
    }

    #[test]
    fn invalid_section_falls_back_to_defaults() {
        let config = deserialize_or_warn(json!({ "no-such-key": true }));
        assert!(config.coverage.is_none());
        assert!(config.skip.is_empty());
    }

    #[test]
    fn workspace_metadata_wins_over_package_metadata() {
        let metadata = metadata_from(json!({
            "metadata": { "lockpick": { "skip": ["audit"] } },
            "packages": [
                { "metadata": { "lockpick": { "skip": ["fmt"] } }, "targets": [] },
            ],
        }));
        let section = extract_lockpick(&metadata).unwrap();
        assert_eq!(section, json!({ "skip": ["audit"] }));
    }

    #[test]
    fn single_package_metadata_is_the_fallback() {
        let metadata = metadata_from(json!({
            "metadata": null,
            "packages": [
                { "metadata": { "lockpick": { "skip": ["fmt"] } }, "targets": [] },
            ],
        }));
        let section = extract_lockpick(&metadata).unwrap();
        assert_eq!(section, json!({ "skip": ["fmt"] }));
    }

    #[test]
    fn multi_package_metadata_without_workspace_section_is_dropped() {
        let metadata = metadata_from(json!({
            "metadata": null,
            "packages": [
                { "metadata": { "lockpick": { "skip": ["fmt"] } }, "targets": [] },
                { "metadata": null, "targets": [] },
            ],
        }));
        assert!(extract_lockpick(&metadata).is_none());
    }
}
