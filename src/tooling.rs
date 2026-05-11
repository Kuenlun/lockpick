// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

// Scaffolding for v1: the install-message constants and the nextest /
// machete / audit detectors land alongside their consumer checks in
// later phases of the refactor.
#![allow(dead_code)]

use std::process::{Command, Stdio};

pub const INSTALL_LLVM_COV: &str = "cargo install cargo-llvm-cov";
pub const INSTALL_NEXTEST: &str = "cargo install cargo-nextest --locked";
pub const INSTALL_MACHETE: &str = "cargo install cargo-machete";
pub const INSTALL_AUDIT: &str = "cargo install cargo-audit";

fn has_cargo_subcommand(subcommand: &str) -> bool {
    Command::new("cargo")
        .args([subcommand, "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[must_use]
pub fn has_llvm_cov() -> bool {
    has_cargo_subcommand("llvm-cov")
}

#[must_use]
pub fn has_nextest() -> bool {
    has_cargo_subcommand("nextest")
}

#[must_use]
pub fn has_machete() -> bool {
    has_cargo_subcommand("machete")
}

#[must_use]
pub fn has_audit() -> bool {
    has_cargo_subcommand("audit")
}
