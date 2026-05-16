// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Lockpick configuration loaded from `[workspace.metadata.lockpick]`
//! (preferred) or `[package.metadata.lockpick]` via `cargo metadata`.

use std::path::PathBuf;
use std::process::{Output, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::tooling::cargo_command;

/// Per-metric coverage thresholds.
///
/// `functions`, `lines`, and `regions` always run and default to 100%.
/// `branches` is `Option<u8>` because branch coverage requires nightly:
/// `None` means "unset" and behaves as 100% when nightly is detected and
/// is silently dropped on stable. `Some(n)` is an explicit user choice
/// and causes lockpick to refuse to run on stable.
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
    pub coverage: CoverageConfig,
}

/// Lockpick config and workspace facts derived from a single
/// `cargo metadata` invocation.
#[derive(Debug, Clone, Default)]
pub struct LockpickMetadata {
    pub config: Config,
    pub has_lib_target: bool,
    /// Absolute path of the enclosing workspace as reported by
    /// `cargo metadata`. `None` when the probe failed (no Cargo.toml
    /// in scope, malformed JSON, …).
    pub workspace_root: Option<PathBuf>,
}

#[derive(Deserialize, Default)]
struct CargoMetadata {
    #[serde(default)]
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

impl LockpickMetadata {
    /// Probe `cargo metadata` and fall back to defaults on any failure.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(run_cargo_metadata())
    }

    fn load_from(metadata: Option<CargoMetadata>) -> Self {
        let Some(metadata) = metadata else {
            return Self::default();
        };
        let has_lib_target = metadata
            .packages
            .iter()
            .flat_map(|p| &p.targets)
            .any(|t| t.kind.iter().any(|k| k == "lib"));
        let config = extract_lockpick(&metadata, &mut |msg| eprintln!("warning: {msg}"))
            .map_or_else(Config::default, deserialize_or_warn);
        Self {
            config,
            has_lib_target,
            workspace_root: metadata.workspace_root,
        }
    }
}

fn deserialize_or_warn(section: Value) -> Config {
    serde_json::from_value(section).unwrap_or_else(|e| {
        eprintln!("warning: invalid [*.metadata.lockpick] section: {e} — using defaults");
        Config::default()
    })
}

fn run_cargo_metadata() -> Option<CargoMetadata> {
    parse_cargo_metadata(
        cargo_command()
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .stderr(Stdio::null())
            .output(),
    )
}

/// Parse a `cargo metadata` spawn result; returns `None` on any failure
/// (spawn error, non-zero exit, or malformed JSON).
fn parse_cargo_metadata(result: std::io::Result<Output>) -> Option<CargoMetadata> {
    let output = result.ok()?;
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
/// the workspace-scoped section get a warning — there is no safe winner to
/// pick workspace-wide, so the configuration is dropped.
fn extract_lockpick(metadata: &CargoMetadata, warn: &mut dyn FnMut(&str)) -> Option<Value> {
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
        warn(&format!(
            "found `[package.metadata.lockpick]` in {stray} package(s) of a multi-crate workspace — use `[workspace.metadata.lockpick]` to apply it workspace-wide"
        ));
    }
    None
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta_with(workspace: Value, packages: Vec<Value>) -> CargoMetadata {
        CargoMetadata {
            workspace_metadata: workspace,
            workspace_root: None,
            packages: packages
                .into_iter()
                .map(|metadata| CargoPackage {
                    metadata,
                    targets: Vec::new(),
                })
                .collect(),
        }
    }

    fn meta_with_targets(targets: Vec<Vec<&str>>) -> CargoMetadata {
        CargoMetadata {
            workspace_metadata: Value::Null,
            workspace_root: None,
            packages: vec![CargoPackage {
                metadata: Value::Null,
                targets: targets
                    .into_iter()
                    .map(|kinds| CargoTarget {
                        kind: kinds.into_iter().map(str::to_string).collect(),
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn coverage_config_defaults_to_100_for_always_on_metrics_and_none_for_branches() {
        let c = CoverageConfig::default();
        assert_eq!(c.functions, 100);
        assert_eq!(c.lines, 100);
        assert_eq!(c.regions, 100);
        // Branches must default to unset so stable Rust users do not
        // hit the nightly-required gate without opting in.
        assert!(c.branches.is_none());
    }

    #[test]
    fn config_default_has_no_license_header() {
        let c = Config::default();
        assert!(c.license_header.is_none());
        assert!(c.license_header_globs.is_none());
        assert_eq!(c.coverage.functions, 100);
        assert!(c.coverage.branches.is_none());
    }

    fn extract_with_warnings(meta: &CargoMetadata) -> (Option<Value>, Vec<String>) {
        let mut warnings: Vec<String> = Vec::new();
        let result = extract_lockpick(meta, &mut |w| warnings.push(w.to_string()));
        (result, warnings)
    }

    #[test]
    fn extract_lockpick_prefers_workspace_metadata() {
        let meta = meta_with(
            json!({ "lockpick": { "license-header": "ws.txt" } }),
            vec![json!({ "lockpick": { "license-header": "pkg.txt" } })],
        );
        let (result, warnings) = extract_with_warnings(&meta);
        let v = result.expect("found");
        assert_eq!(v["license-header"], "ws.txt");
        assert!(warnings.is_empty());
    }

    #[test]
    fn extract_lockpick_falls_back_to_single_package_metadata() {
        let meta = meta_with(
            Value::Null,
            vec![json!({ "lockpick": { "license-header": "pkg.txt" } })],
        );
        let (result, warnings) = extract_with_warnings(&meta);
        let v = result.expect("found");
        assert_eq!(v["license-header"], "pkg.txt");
        assert!(warnings.is_empty());
    }

    #[test]
    fn extract_lockpick_warns_and_returns_none_for_stray_metadata_in_multi_crate_workspace() {
        let meta = meta_with(
            Value::Null,
            vec![
                json!({ "lockpick": { "license-header": "a.txt" } }),
                json!({ "lockpick": { "license-header": "b.txt" } }),
            ],
        );
        let (result, warnings) = extract_with_warnings(&meta);
        assert!(result.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("2 package(s)"));
        assert!(warnings[0].contains("[workspace.metadata.lockpick]"));
    }

    #[test]
    fn extract_lockpick_warns_when_a_single_member_of_a_multi_crate_workspace_has_metadata() {
        let meta = meta_with(
            Value::Null,
            vec![
                json!({ "lockpick": { "license-header": "a.txt" } }),
                json!({ "other": {} }),
            ],
        );
        let (result, warnings) = extract_with_warnings(&meta);
        assert!(result.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("1 package(s)"));
    }

    #[test]
    fn extract_lockpick_is_silent_when_multi_crate_workspace_has_no_lockpick_metadata() {
        let meta = meta_with(
            Value::Null,
            vec![json!({ "other": {} }), json!({ "other": {} })],
        );
        let (result, warnings) = extract_with_warnings(&meta);
        assert!(result.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn extract_lockpick_returns_none_when_section_is_absent() {
        let meta = meta_with(json!({ "other": {} }), vec![json!({ "other": {} })]);
        let (result, warnings) = extract_with_warnings(&meta);
        assert!(result.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn extract_lockpick_returns_none_when_workspace_metadata_lacks_lockpick_key() {
        let meta = meta_with(json!({ "other": "x" }), vec![]);
        let (result, warnings) = extract_with_warnings(&meta);
        assert!(result.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn extract_lockpick_returns_none_when_single_package_metadata_is_not_object() {
        let meta = meta_with(Value::Null, vec![json!("a string, not an object")]);
        let (result, warnings) = extract_with_warnings(&meta);
        assert!(result.is_none());
        assert!(warnings.is_empty());
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
        // Section did not mention `branches`; stays unset.
        assert!(cfg.coverage.branches.is_none());
    }

    #[test]
    fn coverage_defaults_to_unset_branches_and_100_elsewhere_when_section_is_omitted() {
        let v = json!({ "license-header": "header.txt" });
        let cfg: Config = serde_json::from_value(v).unwrap();
        assert_eq!(cfg.coverage.functions, 100);
        assert_eq!(cfg.coverage.lines, 100);
        assert_eq!(cfg.coverage.regions, 100);
        assert!(cfg.coverage.branches.is_none());
    }

    #[test]
    fn coverage_explicit_branches_zero_is_preserved_as_some_zero() {
        let v = json!({ "coverage": { "branches": 0 } });
        let cfg: Config = serde_json::from_value(v).unwrap();
        assert_eq!(cfg.coverage.functions, 100);
        assert_eq!(cfg.coverage.lines, 100);
        assert_eq!(cfg.coverage.regions, 100);
        // Some(0) distinguishes "user disabled the gate" from "user did
        // not configure it" (None); both forms must round-trip cleanly.
        assert_eq!(cfg.coverage.branches, Some(0));
    }

    #[test]
    fn config_rejects_unknown_top_level_key() {
        let v = json!({ "license-header": "hdr.txt", "licens-header": "typo.txt" });
        let err = serde_json::from_value::<Config>(v).expect_err("typo must fail");
        assert!(
            err.to_string().contains("licens-header"),
            "error should name the offending key, got: {err}"
        );
    }

    #[test]
    fn coverage_config_rejects_unknown_key() {
        let v = json!({ "coverage": { "branches": 80, "branchs": 90 } });
        let err = serde_json::from_value::<Config>(v).expect_err("typo must fail");
        assert!(
            err.to_string().contains("branchs"),
            "error should name the offending key, got: {err}"
        );
    }

    #[test]
    fn load_from_none_returns_defaults() {
        let m = LockpickMetadata::load_from(None);
        assert!(m.config.license_header.is_none());
        assert!(!m.has_lib_target);
        assert!(m.workspace_root.is_none());
    }

    #[test]
    fn load_from_surfaces_workspace_root_when_cargo_metadata_provides_one() {
        let m = LockpickMetadata::load_from(Some(CargoMetadata {
            workspace_metadata: Value::Null,
            workspace_root: Some(PathBuf::from("/some/workspace")),
            packages: Vec::new(),
        }));
        assert_eq!(
            m.workspace_root.as_deref(),
            Some(std::path::Path::new("/some/workspace"))
        );
    }

    #[test]
    fn load_from_metadata_without_lockpick_section_returns_defaults() {
        let m = LockpickMetadata::load_from(Some(meta_with(json!({ "other": {} }), vec![])));
        assert!(m.config.license_header.is_none());
    }

    #[test]
    fn load_from_metadata_with_lockpick_section_applies_overrides() {
        let m = LockpickMetadata::load_from(Some(meta_with(
            json!({ "lockpick": { "license-header": "hdr.txt" } }),
            vec![],
        )));
        assert_eq!(
            m.config.license_header.as_deref(),
            Some(std::path::Path::new("hdr.txt"))
        );
    }

    #[test]
    fn load_from_multi_crate_workspace_with_stray_per_package_metadata_falls_back_to_defaults() {
        let m = LockpickMetadata::load_from(Some(meta_with(
            Value::Null,
            vec![
                json!({ "lockpick": { "license-header": "a.txt" } }),
                json!({ "lockpick": { "license-header": "b.txt" } }),
            ],
        )));
        assert!(m.config.license_header.is_none());
    }

    #[test]
    fn load_from_falls_back_to_defaults_on_invalid_section_and_warns() {
        let m = LockpickMetadata::load_from(Some(meta_with(
            json!({ "lockpick": { "coverage": "not a number" } }),
            vec![],
        )));
        assert!(m.config.license_header.is_none());
        assert_eq!(m.config.coverage.functions, 100);
    }

    #[test]
    fn load_from_unknown_top_level_key_falls_back_to_defaults() {
        let m = LockpickMetadata::load_from(Some(meta_with(
            json!({ "lockpick": { "licens-header": "typo.txt" } }),
            vec![],
        )));
        assert!(m.config.license_header.is_none());
        assert_eq!(m.config.coverage.functions, 100);
    }

    #[test]
    fn load_from_unknown_coverage_key_falls_back_to_defaults() {
        let m = LockpickMetadata::load_from(Some(meta_with(
            json!({ "lockpick": { "coverage": { "branchs": 90 } } }),
            vec![],
        )));
        assert!(m.config.coverage.branches.is_none());
        assert_eq!(m.config.coverage.functions, 100);
    }

    #[test]
    fn has_lib_target_is_true_when_any_package_exposes_lib() {
        let m = LockpickMetadata::load_from(Some(meta_with_targets(vec![vec!["lib"]])));
        assert!(m.has_lib_target);
    }

    #[test]
    fn has_lib_target_is_true_when_target_has_multi_kind_including_lib() {
        let m = LockpickMetadata::load_from(Some(meta_with_targets(vec![vec!["lib", "cdylib"]])));
        assert!(m.has_lib_target);
    }

    #[test]
    fn has_lib_target_is_false_for_bin_only_workspace() {
        let m = LockpickMetadata::load_from(Some(meta_with_targets(vec![vec!["bin"]])));
        assert!(!m.has_lib_target);
    }

    #[test]
    fn has_lib_target_is_false_when_no_packages() {
        let m = LockpickMetadata::load_from(Some(meta_with(Value::Null, vec![])));
        assert!(!m.has_lib_target);
    }

    #[test]
    fn load_smoke_test_against_real_cargo_metadata() {
        let m = LockpickMetadata::load();
        assert!(!m.has_lib_target);
        // The smoke test runs inside the lockpick workspace, so the
        // probe must surface a root for the runner to anchor cwd on.
        assert!(
            m.workspace_root.is_some(),
            "real `cargo metadata` should surface a workspace_root"
        );
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
}
