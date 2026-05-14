// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockpickError {
    #[error("{0} check(s) failed")]
    ChecksFailed(usize),

    #[error("required tool `{tool}` is not installed.\nInstall it with: {install}")]
    MissingTool {
        tool: &'static str,
        install: &'static str,
    },
}
