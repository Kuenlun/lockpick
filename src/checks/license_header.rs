// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! License-header check. Verifies that every file matched by the
//! configured globs starts with the bytes of a canonical header file.
//! Files whose first line contains `@generated` are skipped.
//!
//! Opt-in: only runs when `[*.metadata.lockpick] license-header = "..."`
//! is set in `Cargo.toml`.

use std::fs;
use std::path::PathBuf;

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

/// Per-file classification used by the scan loop. Pulling this out of the
/// loop body keeps the IO error branch unit-testable cross-platform
/// without needing to actually produce an unreadable file at FS level.
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
        format!("license-header against `{}`", self.header_path.display())
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

        let mut offenders: Vec<PathBuf> = Vec::new();
        let mut scanned = 0_usize;
        for file in files {
            if file == self.header_path {
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
    Ok(files)
}

fn is_generated(contents: &[u8]) -> bool {
    let first_line = contents.split(|&b| b == b'\n').next().unwrap_or(b"");
    let line = String::from_utf8_lossy(first_line);
    line.contains("@generated")
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Returns a unique sub-tempdir per test so parallel runs don't collide.
    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lockpick_{tag}_{pid}_{nonce}",
            pid = std::process::id(),
            nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_generated_marker_on_first_line() {
        assert!(is_generated(b"// @generated by prost\nfn foo() {}"));
        assert!(is_generated(b"//! @generated\n"));
        assert!(!is_generated(
            b"// SPDX-License-Identifier: MIT\n@generated"
        ));
    }

    #[test]
    fn is_generated_returns_false_for_empty_content() {
        assert!(!is_generated(b""));
    }

    #[test]
    fn cmd_describes_the_header_being_enforced() {
        let check = LicenseHeaderCheck {
            header_path: PathBuf::from(".github/license_header.rs"),
            globs: vec![],
        };
        assert!(check.cmd().contains(".github/license_header.rs"));
    }

    #[test]
    fn classify_returns_offender_for_io_error() {
        let result = classify(Err(io::Error::other("denied")), b"// HEADER\n");
        assert_eq!(result, Classification::Offender);
    }

    #[test]
    fn classify_returns_generated_for_first_line_marker() {
        let result = classify(Ok(b"// @generated by prost\nfn x() {}".to_vec()), b"// H\n");
        assert_eq!(result, Classification::Generated);
    }

    #[test]
    fn classify_returns_match_when_contents_start_with_header() {
        let result = classify(Ok(b"// HEADER\nfn x() {}".to_vec()), b"// HEADER\n");
        assert_eq!(result, Classification::Match);
    }

    #[test]
    fn classify_returns_offender_when_contents_lack_header() {
        let result = classify(Ok(b"fn x() {}".to_vec()), b"// HEADER\n");
        assert_eq!(result, Classification::Offender);
    }

    #[test]
    fn run_fails_when_header_file_is_missing() {
        let check = LicenseHeaderCheck {
            header_path: PathBuf::from("/definitely/does/not/exist.txt"),
            globs: vec![],
        };
        let outcome = check.run(&FakeRunner::passing());
        assert!(outcome.failed());
        assert!(outcome.output.contains("could not read"));
    }

    #[test]
    fn run_fails_when_header_file_is_empty() {
        let dir = tempdir("empty");
        let header = dir.join("h.txt");
        std::fs::write(&header, b"").unwrap();
        let check = LicenseHeaderCheck {
            header_path: header,
            globs: vec![],
        };
        let outcome = check.run(&FakeRunner::passing());
        assert!(outcome.failed());
        assert!(outcome.output.contains("empty"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_fails_on_bad_glob_pattern() {
        let dir = tempdir("badglob");
        let header = dir.join("h.txt");
        std::fs::write(&header, b"// HEADER\n").unwrap();
        let check = LicenseHeaderCheck {
            header_path: header,
            globs: vec!["a/***/b".to_string()],
        };
        let outcome = check.run(&FakeRunner::passing());
        assert!(outcome.failed());
        assert!(outcome.output.to_lowercase().contains("glob"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_passes_when_every_scanned_file_starts_with_header() {
        let dir = tempdir("ok");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let header = dir.join("h.txt");
        std::fs::write(&header, b"// HEADER\n").unwrap();
        std::fs::write(dir.join("src").join("a.rs"), b"// HEADER\nfn main() {}\n").unwrap();
        std::fs::write(dir.join("src").join("b.rs"), b"// HEADER\nfn other() {}\n").unwrap();
        let check = LicenseHeaderCheck {
            header_path: header,
            globs: vec![format!("{}/src/*.rs", dir.display())],
        };
        let outcome = check.run(&FakeRunner::passing());
        assert!(outcome.passed(), "got: {}", outcome.output);
        assert!(outcome.output.contains("2 file(s) checked"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_lists_offenders_with_paths() {
        let dir = tempdir("bad");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let header = dir.join("h.txt");
        std::fs::write(&header, b"// HEADER\n").unwrap();
        std::fs::write(dir.join("src").join("ok.rs"), b"// HEADER\nfn ok() {}\n").unwrap();
        std::fs::write(dir.join("src").join("bad.rs"), b"fn bad() {}\n").unwrap();
        let check = LicenseHeaderCheck {
            header_path: header,
            globs: vec![format!("{}/src/*.rs", dir.display())],
        };
        let outcome = check.run(&FakeRunner::passing());
        assert!(outcome.failed());
        assert!(outcome.output.contains("bad.rs"));
        assert!(outcome.output.contains("expected header"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_skips_generated_files() {
        let dir = tempdir("gen");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let header = dir.join("h.txt");
        std::fs::write(&header, b"// HEADER\n").unwrap();
        std::fs::write(dir.join("src").join("real.rs"), b"// HEADER\nfn x() {}\n").unwrap();
        std::fs::write(
            dir.join("src").join("gen.rs"),
            b"// @generated by prost\nfn g() {}\n",
        )
        .unwrap();
        let check = LicenseHeaderCheck {
            header_path: header,
            globs: vec![format!("{}/src/*.rs", dir.display())],
        };
        let outcome = check.run(&FakeRunner::passing());
        assert!(outcome.passed(), "got: {}", outcome.output);
        assert!(outcome.output.contains("1 file(s) checked"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_skips_the_header_file_itself_when_it_matches_the_glob() {
        let dir = tempdir("selfheader");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let header = dir.join("src").join("header.txt");
        std::fs::write(&header, b"// HEADER\n").unwrap();
        std::fs::write(dir.join("src").join("a.rs"), b"// HEADER\nfn a() {}\n").unwrap();
        let check = LicenseHeaderCheck {
            header_path: header,
            globs: vec![format!("{}/src/*", dir.display())],
        };
        let outcome = check.run(&FakeRunner::passing());
        assert!(outcome.passed(), "got: {}", outcome.output);
        assert!(outcome.output.contains("1 file(s) checked"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_files_ignores_directory_glob_matches() {
        let dir = tempdir("dirglob");
        std::fs::create_dir_all(dir.join("src/nested")).unwrap();
        std::fs::write(dir.join("src").join("a.rs"), b"// hi\n").unwrap();
        let pattern = format!("{}/src/*", dir.display());
        let collected = collect_files(&[pattern]).unwrap();
        assert_eq!(collected.len(), 1);
        assert!(collected[0].is_file());
        std::fs::remove_dir_all(&dir).ok();
    }
}
