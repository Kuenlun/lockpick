// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Lockpick configuration loaded from `[workspace.metadata.lockpick]`
//! or `[package.metadata.lockpick]` in `Cargo.toml`.
//!
//! In the v1 foundation phase this is a skeleton with sensible defaults;
//! the actual `cargo metadata` parser is wired up alongside the coverage
//! rework so that nothing in the current behavior changes.

#![allow(dead_code)]

use std::path::PathBuf;

/// Per-metric coverage thresholds. Defaults to 100% on every metric.
#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Default, Clone)]
pub struct Config {
    pub license_header: Option<PathBuf>,
    pub license_header_globs: Option<Vec<String>>,
    pub coverage: CoverageConfig,
}

impl Config {
    /// Load lockpick configuration. The full `cargo metadata` parser lands
    /// with the coverage rework; for now we return defaults so the new
    /// plumbing is exercised without changing behavior.
    #[must_use]
    pub fn load() -> Self {
        Self::default()
    }
}
