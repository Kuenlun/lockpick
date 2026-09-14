// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Lockpick configuration loaded from `[workspace.metadata.lockpick]`
//! (preferred) or `[package.metadata.lockpick]` via `cargo metadata`.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::cli::SkipOption;
use crate::error::LockpickError;
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
pub(crate) struct CoverageConfig {
    pub(crate) functions: u8,
    pub(crate) lines: u8,
    pub(crate) regions: u8,
    pub(crate) branches: Option<u8>,
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
pub(crate) struct Config {
    /// Require an existing, current lockfile for Cargo build commands.
    pub(crate) locked: bool,
    /// Override Cargo's default compilation target with the host.
    pub(crate) host_target: bool,
    /// Additional compilation and lint checks, without executing tests.
    pub(crate) target_checks: Vec<TargetCheck>,
    pub(crate) license_header: Option<PathBuf>,
    pub(crate) license_header_globs: Option<Vec<String>>,
    /// Opt-in coverage gate. `Some` whenever the
    /// `[*.metadata.lockpick.coverage]` table exists, even empty, with
    /// per-metric thresholds defaulting to 100%. `None` keeps coverage
    /// off unless the CLI passes `--coverage`.
    pub(crate) coverage: Option<CoverageConfig>,
    /// Project-wide skip list. Same kebab-case identifiers `--skip`
    /// accepts on the CLI, merged with (not replaced by) any CLI flags.
    pub(crate) skip: Vec<SkipOption>,
}

/// Cargo artifact selection for an additional target check.
#[derive(Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Artifacts {
    Lib,
    Bins,
    #[default]
    AllTargets,
}

impl Artifacts {
    pub(crate) const fn flag(self) -> &'static str {
        match self {
            Self::Lib => "--lib",
            Self::Bins => "--bins",
            Self::AllTargets => "--all-targets",
        }
    }
}

/// One additional Clippy invocation. The default matches the ordinary gate.
#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct TargetCheck {
    pub(crate) target: Option<String>,
    pub(crate) profile: String,
    pub(crate) artifacts: Artifacts,
    pub(crate) packages: Vec<String>,
    pub(crate) all_features: bool,
    pub(crate) no_default_features: bool,
    pub(crate) features: Vec<String>,
}

impl Default for TargetCheck {
    fn default() -> Self {
        Self {
            target: None,
            profile: "dev".into(),
            artifacts: Artifacts::AllTargets,
            packages: Vec::new(),
            all_features: true,
            no_default_features: false,
            features: Vec::new(),
        }
    }
}

impl TargetCheck {
    fn validate(&mut self) -> Result<(), LockpickError> {
        if self.all_features && (self.no_default_features || !self.features.is_empty()) {
            return Err(LockpickError::Configuration(
                "target-checks: set all-features = false when selecting features or disabling defaults".into(),
            ));
        }
        for value in self
            .target
            .iter()
            .chain(std::iter::once(&self.profile))
            .chain(&self.packages)
            .chain(&self.features)
        {
            if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
                return Err(LockpickError::Configuration(
                    "target-checks: targets, profiles, packages and features must be nonempty values without control characters or leading '-'".into(),
                ));
            }
        }
        self.packages.sort();
        self.packages.dedup();
        self.features.sort();
        self.features.dedup();
        Ok(())
    }
}

/// Lockpick config and workspace facts derived from a single
/// `cargo metadata` invocation.
#[derive(Debug, Clone, Default)]
pub(crate) struct LockpickMetadata {
    pub(crate) target_directory: Option<PathBuf>,
    pub(crate) config: Config,
    pub(crate) has_lib_target: bool,
    /// Absolute enclosing workspace path, populated after a successful probe.
    pub(crate) workspace_root: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
struct CargoMetadata {
    #[serde(default)]
    target_directory: Option<PathBuf>,
    // Cargo emits this key as plain `metadata`, not `workspace_metadata`.
    #[serde(default, rename = "metadata")]
    workspace_metadata: Value,
    workspace_root: PathBuf,
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize, Default)]
struct CargoPackage {
    #[serde(default)]
    manifest_path: PathBuf,
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
    /// Load workspace facts and validate configuration before any checks or fixes run.
    pub(crate) fn load() -> Result<Self, LockpickError> {
        let metadata = run_cargo_metadata()?;
        let has_lib_target = metadata
            .packages
            .iter()
            .flat_map(|p| &p.targets)
            .any(|t| t.kind.iter().any(|k| LIB_KINDS.contains(&k.as_str())));
        let mut config =
            extract_lockpick(&metadata)?.map_or_else(|| Ok(Config::default()), parse_config)?;
        resolve_license_paths(&mut config, &metadata);
        Ok(Self {
            target_directory: metadata.target_directory,
            config,
            has_lib_target,
            workspace_root: Some(metadata.workspace_root),
        })
    }
}

fn parse_config(section: Value) -> Result<Config, LockpickError> {
    let mut config: Config =
        serde_json::from_value(section).map_err(|e| LockpickError::Configuration(e.to_string()))?;
    if let Some(coverage) = config.coverage {
        for (name, threshold) in [
            ("functions", coverage.functions),
            ("lines", coverage.lines),
            ("regions", coverage.regions),
            ("branches", coverage.branches.unwrap_or(100)),
        ] {
            if threshold > 100 {
                return Err(LockpickError::Configuration(format!(
                    "coverage.{name} must be between 0 and 100, got {threshold}"
                )));
            }
        }
    }
    for check in &mut config.target_checks {
        check.validate()?;
    }
    Ok(config)
}

/// License paths have the same workspace anchor as Cargo subprocesses.
fn resolve_license_paths(config: &mut Config, metadata: &CargoMetadata) {
    let root = &metadata.workspace_root;
    if let Some(header) = &mut config.license_header {
        *header = root.join(&*header);
        let patterns = config.license_header_globs.take().unwrap_or_else(|| {
            metadata
                .packages
                .iter()
                .filter_map(|package| package.manifest_path.parent())
                .flat_map(|directory| {
                    crate::checks::license_header::default_globs()
                        .into_iter()
                        .map(move |pattern| {
                            format!(
                                "{}/{}",
                                glob::Pattern::escape(&directory.to_string_lossy()),
                                pattern
                            )
                        })
                })
                .collect()
        });
        config.license_header_globs = Some(
            patterns
                .into_iter()
                .map(|pattern| {
                    if std::path::Path::new(&pattern).is_absolute() {
                        pattern
                    } else {
                        format!(
                            "{}/{}",
                            glob::Pattern::escape(&root.to_string_lossy()),
                            pattern
                        )
                    }
                })
                .collect(),
        );
    }
}

fn run_cargo_metadata() -> Result<CargoMetadata, LockpickError> {
    let output = cargo_command()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| LockpickError::Configuration(format!("could not run cargo metadata: {e}")))?;
    if !output.status.success() {
        return Err(LockpickError::Configuration(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| LockpickError::Configuration(format!("invalid cargo metadata JSON: {e}")))
}

/// Locate `[*.metadata.lockpick]` in priority order:
///
/// 1. `[workspace.metadata.lockpick]`.
/// 2. `[package.metadata.lockpick]` of a single-package workspace.
///
/// Multi-package workspaces that set `[package.metadata.lockpick]` without
/// the workspace-scoped section are rejected: there is no safe winner to pick workspace-wide.
fn extract_lockpick(metadata: &CargoMetadata) -> Result<Option<Value>, LockpickError> {
    fn lockpick_in(value: &Value) -> Option<Value> {
        value.as_object().and_then(|m| m.get("lockpick")).cloned()
    }
    if let Some(ws) = lockpick_in(&metadata.workspace_metadata) {
        return Ok(Some(ws));
    }
    if let [package] = metadata.packages.as_slice() {
        return Ok(lockpick_in(&package.metadata));
    }
    let stray = metadata
        .packages
        .iter()
        .filter(|p| lockpick_in(&p.metadata).is_some())
        .count();
    if stray > 0 {
        return Err(LockpickError::Configuration(format!(
            "found `[package.metadata.lockpick]` in {stray} package(s) of a multi-crate workspace. Use `[workspace.metadata.lockpick]` to apply it workspace-wide"
        )));
    }
    Ok(None)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cli::SkipOption;

    fn metadata_from(value: Value) -> CargoMetadata {
        let mut value = value;
        let _previous = value
            .as_object_mut()
            .unwrap()
            .insert("workspace_root".into(), json!("/workspace"));
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
            "coverage": { "lines": 90_i32 },
        }))
        .unwrap();
        assert_eq!(config.license_header.unwrap(), PathBuf::from("hdr.txt"));
        assert_eq!(config.skip, vec![SkipOption::Audit, SkipOption::Machete]);
        assert_eq!(config.coverage.unwrap().lines, 90);
    }

    #[test]
    fn invalid_sections_and_thresholds_are_rejected() {
        for value in [
            json!({ "no-such-key": true }),
            json!({ "coverage": { "line": 100_i32 } }),
            json!({ "skip": ["unknown"] }),
            json!({ "coverage": { "lines": -1_i32 } }),
            json!({ "coverage": { "lines": 99.5_f64 } }),
        ] {
            assert!(parse_config(value).is_err());
        }
        for metric in ["functions", "lines", "regions", "branches"] {
            for threshold in [0_i32, 100_i32, 101_i32, 255_i32] {
                let result = parse_config(json!({ "coverage": { metric: threshold } }));
                assert_eq!(
                    result.is_ok(),
                    threshold <= 100_i32,
                    "{metric}: {threshold}"
                );
            }
        }
    }

    #[test]
    fn workspace_metadata_wins_over_package_metadata() {
        let metadata = metadata_from(json!({
            "metadata": { "lockpick": { "skip": ["audit"] } },
            "packages": [
                { "metadata": { "lockpick": { "skip": ["fmt"] } }, "targets": [] },
            ],
        }));
        let section = extract_lockpick(&metadata).unwrap().unwrap();
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
        let section = extract_lockpick(&metadata).unwrap().unwrap();
        assert_eq!(section, json!({ "skip": ["fmt"] }));
    }

    #[test]
    fn multi_package_metadata_without_workspace_section_is_rejected() {
        let metadata = metadata_from(json!({
            "metadata": null,
            "packages": [
                { "metadata": { "lockpick": { "skip": ["fmt"] } }, "targets": [] },
                { "metadata": null, "targets": [] },
            ],
        }));
        assert!(extract_lockpick(&metadata).is_err());
    }
}
