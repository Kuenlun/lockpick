// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! License-header byte-equality check. Opt-in via
//! `[*.metadata.lockpick] license-header = "..."`. Skips files whose
//! first lines contain `@generated`.

use std::fs;
use std::path::{Path, PathBuf};

use super::{Check, Runner};
use crate::reporter::{CheckOutcome, TaskStatus};

pub struct LicenseHeaderCheck {
    pub header_path: PathBuf,
    pub globs: Vec<String>,
}

#[must_use]
pub fn default_globs() -> Vec<String> {
    vec![
        "src/**/*.rs".to_string(),
        "tests/**/*.rs".to_string(),
        "examples/**/*.rs".to_string(),
        "benches/**/*.rs".to_string(),
    ]
}

#[derive(Debug, PartialEq, Eq)]
enum Classification {
    Match,
    Generated,
    Offender,
}

fn classify(contents: std::io::Result<Vec<u8>>, header: &[u8]) -> Classification {
    let Ok(contents) = contents else {
        return Classification::Offender;
    };
    if is_generated(&contents) {
        return Classification::Generated;
    }
    if contents.starts_with(header) {
        Classification::Match
    } else {
        Classification::Offender
    }
}

impl Check for LicenseHeaderCheck {
    fn label(&self) -> &'static str {
        "license"
    }

    fn cmd(&self) -> String {
        // `(in-process)` flags this as not a shell-runnable command,
        // unlike every other check whose `cmd` is a cargo line.
        format!(
            "(in-process) license-header against `{}`",
            self.header_path.display()
        )
    }

    fn chain_position(&self) -> Option<u8> {
        None
    }

    fn run(&self, _runner: &dyn Runner) -> CheckOutcome {
        let header = match fs::read(&self.header_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                return CheckOutcome {
                    status: TaskStatus::Fail,
                    output: format!(
                        "could not read license header file `{}`: {e}",
                        self.header_path.display()
                    ),
                };
            }
        };
        if header.is_empty() {
            return CheckOutcome {
                status: TaskStatus::Fail,
                output: format!(
                    "license header file `{}` is empty",
                    self.header_path.display()
                ),
            };
        }

        let files = match collect_files(&self.globs) {
            Ok(f) => f,
            Err(e) => {
                return CheckOutcome {
                    status: TaskStatus::Fail,
                    output: format!("invalid glob in license-header-globs: {e}"),
                };
            }
        };

        // Canonicalize so the header file does not flag itself when a
        // glob picks it up under a different path spelling.
        let header_key = normalize(&self.header_path);

        let mut offenders: Vec<PathBuf> = Vec::new();
        let mut scanned = 0_usize;
        for file in files {
            if normalize(&file) == header_key {
                continue;
            }
            match classify(fs::read(&file), &header) {
                Classification::Match => scanned += 1,
                Classification::Generated => {}
                Classification::Offender => offenders.push(file),
            }
        }

        if offenders.is_empty() {
            return CheckOutcome {
                status: TaskStatus::Pass,
                output: format!(
                    "{scanned} file(s) checked against `{}`",
                    self.header_path.display()
                ),
            };
        }

        let mut lines: Vec<String> = vec!["files missing the expected license header:".to_string()];
        for path in &offenders {
            lines.push(format!("  - {}", path.display()));
        }
        lines.push(String::new());
        lines.push(format!(
            "expected header (from `{}`):",
            self.header_path.display()
        ));
        for line in String::from_utf8_lossy(&header).lines() {
            lines.push(format!("  | {line}"));
        }
        CheckOutcome {
            status: TaskStatus::Fail,
            output: lines.join("\n"),
        }
    }
}

/// Canonicalize for equality comparison, falling back to the raw path
/// when canonicalization fails.
fn normalize(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn collect_files(patterns: &[String]) -> Result<Vec<PathBuf>, glob::PatternError> {
    let mut files = Vec::new();
    for pattern in patterns {
        for entry in glob::glob(pattern)? {
            match entry {
                Ok(path) if path.is_file() => files.push(path),
                _ => {}
            }
        }
    }
    // Dedup so overlapping globs do not scan twice and the offender
    // list stays deterministic.
    files.sort();
    files.dedup();
    Ok(files)
}

/// Leading-line window for the `@generated` marker. Five is enough to
/// catch banners or modelines above the marker.
const GENERATED_HEADER_SCAN_LINES: usize = 5;

fn is_generated(contents: &[u8]) -> bool {
    contents
        .split(|&b| b == b'\n')
        .take(GENERATED_HEADER_SCAN_LINES)
        .any(|line| String::from_utf8_lossy(line).contains("@generated"))
}
