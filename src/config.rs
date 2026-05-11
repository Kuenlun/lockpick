// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Lockpick configuration loaded from `[workspace.metadata.lockpick]`
//! or `[package.metadata.lockpick]` in `Cargo.toml`. Read transparently
//! via `cargo metadata --format-version 1 --no-deps`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

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
            log::warn!("invalid [*.metadata.lockpick] section: {e} — using defaults");
            Self::default()
        })
    }
}

fn run_cargo_metadata() -> Option<CargoMetadata> {
    let output = Command::new("cargo")
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
