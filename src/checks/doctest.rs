// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use std::process::{Output, Stdio};

use super::{Check, Runner, cargo_outcome, fmt_cargo_cmd};
use crate::reporter::CheckOutcome;
use crate::tooling::cargo_command;

const DOCTEST_ARGS: &[&str] = &["--doc", "--workspace", "--all-features"];

pub struct DocTestCheck;

impl Check for DocTestCheck {
    fn label(&self) -> &'static str {
        "doc test"
    }

    fn cmd(&self) -> String {
        fmt_cargo_cmd("test", DOCTEST_ARGS)
    }

    fn run(&self, runner: &dyn Runner) -> CheckOutcome {
        cargo_outcome(runner, "test", DOCTEST_ARGS)
    }
}

/// Returns `true` when any workspace member exposes a `lib` target.
/// Skipping the doc-test check on bin-only workspaces avoids an opaque
/// error from cargo.
#[must_use]
pub fn workspace_has_lib_target() -> bool {
    has_lib_target_in(&cargo_metadata_stdout().unwrap_or_default())
}

/// Pure detector that scans a `cargo metadata` JSON blob for a `lib` kind
/// entry. Factored out so callers can unit-test both outcomes without
/// having to spawn cargo or build a JSON-emitting workspace. The
/// substring deliberately omits the closing `]` so a multi-kind target
/// like `["lib","cdylib"]` is still recognised; cargo emits the array
/// in compact form so spacing is stable.
#[must_use]
pub fn has_lib_target_in(metadata_json: &str) -> bool {
    metadata_json.contains(r#""kind":["lib""#)
}

fn cargo_metadata_stdout() -> Option<String> {
    decode_metadata_stdout(
        cargo_command()
            .args(["metadata", "--no-deps", "--format-version", "1"])
            .stderr(Stdio::null())
            .output(),
    )
}

/// Pure helper: lower a `cargo metadata` spawn result into UTF-8 stdout.
/// Returns `None` for every error path the production code already
/// silently tolerates (spawn failed, stdout was not valid UTF-8).
fn decode_metadata_stdout(result: std::io::Result<Output>) -> Option<String> {
    let output = result.ok()?;
    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::FakeRunner;

    #[test]
    fn label_is_doc_test() {
        assert_eq!(DocTestCheck.label(), "doc test");
    }

    #[test]
    fn cmd_runs_cargo_test_doc() {
        let cmd = DocTestCheck.cmd();
        assert!(cmd.starts_with("cargo test "));
        assert!(cmd.contains("--doc"));
        assert!(cmd.contains("--workspace"));
        assert!(cmd.contains("--all-features"));
    }

    #[test]
    fn run_invokes_cargo_test_with_doc_args() {
        let fake = FakeRunner::passing();
        assert!(DocTestCheck.run(&fake).passed());
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls[0].sub, "test");
        assert!(calls[0].args.contains(&"--doc".to_string()));
    }

    #[test]
    fn has_lib_target_in_detects_lib_kind() {
        let metadata = r#"{"packages":[{"targets":[{"kind":["lib"],"name":"foo"}]}]}"#;
        assert!(has_lib_target_in(metadata));
    }

    #[test]
    fn has_lib_target_in_returns_false_for_bin_only_workspace() {
        let metadata = r#"{"packages":[{"targets":[{"kind":["bin"],"name":"foo"}]}]}"#;
        assert!(!has_lib_target_in(metadata));
    }

    #[test]
    fn has_lib_target_in_detects_lib_inside_a_multi_kind_target() {
        // A single target with `crate-type = ["lib", "cdylib"]` is emitted
        // by cargo as `"kind":["lib","cdylib"]`. The detector must still
        // recognise the lib entry.
        let metadata = r#"{"packages":[{"targets":[{"kind":["lib","cdylib"],"name":"foo"}]}]}"#;
        assert!(has_lib_target_in(metadata));
    }

    #[test]
    fn has_lib_target_in_returns_false_on_empty_input() {
        assert!(!has_lib_target_in(""));
    }

    #[test]
    fn workspace_has_lib_target_does_not_panic() {
        // Smoke-test the production wrapper. The boolean depends on the
        // cwd at test time, so we don't assert on it.
        let _ = workspace_has_lib_target();
    }

    #[test]
    fn decode_metadata_stdout_returns_none_when_spawn_failed() {
        let err: std::io::Result<std::process::Output> = Err(std::io::Error::other("ENOENT"));
        assert!(decode_metadata_stdout(err).is_none());
    }

    #[test]
    fn decode_metadata_stdout_returns_some_for_utf8_stdout() {
        let out = std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .expect("cargo runs");
        let s = decode_metadata_stdout(Ok(out)).expect("utf8");
        assert!(s.starts_with("cargo"));
    }

    #[test]
    fn decode_metadata_stdout_returns_none_for_non_utf8_stdout() {
        // Spawn a trivial command to obtain a real `ExitStatus`, then swap in
        // bytes that aren't valid UTF-8. Going through a shell tool like
        // `printf` is non-portable — BSD `printf` on macOS doesn't grok `\xHH`.
        let mut out = std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .expect("cargo runs");
        out.stdout = vec![0xff, 0xfe];
        assert!(decode_metadata_stdout(Ok(out)).is_none());
    }
}
