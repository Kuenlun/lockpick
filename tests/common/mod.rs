// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

#![expect(
    clippy::redundant_pub_crate,
    reason = "Explicit internal visibility satisfies unreachable_pub."
)]
// `feature(coverage_attribute)` is declared by each test binary at its
// own crate root; this submodule only opts out of instrumentation.
#![cfg_attr(coverage_nightly, coverage(off))]
#![expect(
    clippy::unwrap_used,
    reason = "Fixture setup failures abort their test."
)]
// Not every test binary exercises every helper.
#![expect(
    dead_code,
    reason = "Each integration binary uses only part of the shared fixture helpers."
)]

//! Shared scaffolding for the integration suite: fixture text, Cargo.toml
//! templating, scratch crate scaffolding, and a `Command` factory that
//! quarantines lockpick from the harness env. Unix-only helpers (PATH
//! sanitiser, symlink layout) gate themselves with `#[cfg(unix)]`.

// Cargo passes application dependencies to each integration binary. The CLI is
// exercised through subprocesses, so declare those inherited dependencies here.
#[cfg(unix)]
use signal_hook as _;
use {
    clap as _, clap_cargo as _, clap_complete as _, colored as _, glob as _, indicatif as _,
    serde as _, serde_json as _, thiserror as _,
};

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(crate) type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Matches `rustfmt` output byte-for-byte. Used as the pristine baseline
/// for fmt-check assertions and as the body of well-formed fixtures.
pub(crate) const FORMATTED_MAIN_RS: &str = "fn main() {\n    println!(\"Hello!\");\n}\n";

/// Compiles cleanly but fails `cargo fmt --check`.
pub(crate) const UNFORMATTED_MAIN_RS: &str = "fn main(){println!(\"Hello!\");}\n";

/// Fails `cargo check` (an `&str` cannot bind to `i32`).
pub(crate) const BROKEN_MAIN_RS: &str = "fn main() {\n    let _x: i32 = \"not a number\";\n}\n";

/// Render a `Cargo.toml` body that survives strict clippy out of the
/// box (every field `cargo_common_metadata` demands is present). `extra`
/// is appended after a blank line, ready for `[package.metadata.*]`
/// stanzas.
#[must_use]
pub(crate) fn cargo_toml_strict(name: &str, extra: &str) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\
         description = \"Integration-test fixture for lockpick\"\n\
         license = \"MIT OR Apache-2.0\"\n\
         repository = \"https://example.invalid/{name}\"\n\
         readme = \"README.md\"\n\
         keywords = [\"test\"]\n\
         categories = [\"development-tools\"]\n\
         \n\
         {extra}"
    )
}

/// Scaffold a temp crate at a fresh [`tempfile::TempDir`]. Writes a
/// strict Cargo.toml, an empty README, and every `(relpath, body)` in
/// `files`. Intermediate directories are created on demand.
#[must_use]
pub(crate) fn scratch_crate(
    name: &str,
    extra_toml: &str,
    files: &[(&str, &str)],
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "Cargo.toml",
        &cargo_toml_strict(name, extra_toml),
    );
    write_file(dir.path(), "README.md", "");
    for (rel, body) in files {
        write_file(dir.path(), rel, body);
    }
    dir
}

/// Minimal Cargo project with a well-formed `src/main.rs` so every
/// check that does not require external fixtures (audit, machete,
/// coverage) is green by default.
#[must_use]
pub(crate) fn dummy_cargo_project() -> tempfile::TempDir {
    scratch_crate("dummy_project", "", &[("src/main.rs", FORMATTED_MAIN_RS)])
}

/// Absolute path of the lockpick binary built by the current
/// `cargo test` invocation. The env var is injected by cargo.
#[must_use]
pub(crate) fn lockpick_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lockpick"))
}

/// Env vars the rustup shim chain needs to reach the active toolchain.
/// `HOME` and `CARGO_HOME` locate cargo and rustup state, `RUSTUP_HOME`
/// the toolchain layout, `RUSTUP_TOOLCHAIN` honours `rust-toolchain.toml`
/// overrides set by the harness. `PATH` is repopulated by the caller
/// (either kept verbatim or replaced with a sanitised one). The Windows
/// block forwards what MSVC's `link.exe` (`LIB`, `INCLUDE`, `LIBPATH`)
/// and the Win32 runtime (`SystemRoot`, `TEMP`, `USERPROFILE`, etc.)
/// demand. `forward_env` silently drops any var unset on the current
/// platform, so Unix runs ignore the Windows tail.
const PASSTHROUGH_ENV: &[&str] = &[
    "HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "TMPDIR",
    "USER",
    "LOGNAME",
    // Forward `cargo llvm-cov`'s profraw sink so the instrumented child
    // writes into `target/llvm-cov-target/` instead of dumping
    // `default_*.profraw` in whatever cwd the test happened to pick.
    "LLVM_PROFILE_FILE",
    // Windows MSVC toolchain + Win32 runtime.
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ComSpec",
    "PATHEXT",
];

/// Build a `Command` that runs the lockpick test binary with a tightly
/// scoped env: cleared first, then `PATH` plus the rustup shim chain
/// re-exported from the harness. Override by calling `.env("PATH", ...)`
/// on the returned `Command` (e.g. with [`sanitized_path`]).
#[must_use]
pub(crate) fn run_lockpick(cwd: &Path) -> Command {
    let mut cmd = Command::new(lockpick_bin());
    let _command = cmd.env_clear();
    forward_env(&mut cmd, "PATH");
    for key in PASSTHROUGH_ENV {
        forward_env(&mut cmd, key);
    }
    let _command = cmd.current_dir(cwd);
    cmd
}

/// Re-export `key` from the harness env into `cmd`, if set. Silent
/// no-op on absent vars so callers stay infallible.
pub(crate) fn forward_env(cmd: &mut Command, key: &str) {
    if let Some(val) = std::env::var_os(key) {
        let _command = cmd.env(key, val);
    }
}

/// Cargo plugins lockpick treats as optional. They drive the
/// missing-tool arm of `require_tooling`, so the sanitised PATH must
/// hide every one of them.
const HIDDEN_CARGO_PLUGINS: &[&str] = &[
    "cargo-llvm-cov",
    "cargo-machete",
    "cargo-audit",
    "cargo-nextest",
];

/// Build a tempdir-backed bin directory mirroring the harness PATH but
/// with [`HIDDEN_CARGO_PLUGINS`] excluded, and return that single dir
/// as the new PATH.
///
/// The full mirror is what lets cargo find `cc`, `ld`, `ar`, the rustup
/// shim chain and everything else it spawns mid-pipeline. Hiding only
/// the named plugins keeps `require_tooling` honest without crippling
/// the optional checks the test does NOT skip.
#[cfg(unix)]
pub(crate) fn sanitized_path() -> Result<(tempfile::TempDir, String), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin)?;
    let path = std::env::var_os("PATH").ok_or("PATH unset")?;
    for src_dir in std::env::split_paths(&path) {
        mirror_dir(&src_dir, &bin)?;
    }
    let bin_str = bin
        .to_str()
        .ok_or("sanitized PATH contains non-UTF8 bytes")?
        .to_owned();
    Ok((dir, bin_str))
}

/// Symlink every executable entry of `src` into `dst`, skipping the
/// hidden plugin list and any name `dst` already has (first PATH entry
/// wins, matching shell lookup order).
///
/// `entry.metadata()` returns the symlink's own metadata, not the
/// target's, so `std::fs::metadata` is what tells us whether the
/// resolved path is a real file (catches `/usr/bin/x86_64-linux-gnu-gcc
/// → gcc`, the linker tuple cargo's test phase reaches for).
#[cfg(unix)]
fn mirror_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(src) else {
        // Non-existent or unreadable PATH entries are dropped; the shell
        // tolerates them too.
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if HIDDEN_CARGO_PLUGINS.contains(&name_str) {
            continue;
        }
        let target = dst.join(&name);
        if target.exists() {
            continue;
        }
        let src_path = entry.path();
        let Ok(meta) = std::fs::metadata(&src_path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        std::os::unix::fs::symlink(&src_path, target)?;
    }
    Ok(())
}

#[must_use]
pub(crate) fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[must_use]
pub(crate) fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[must_use]
pub(crate) fn combined(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}

/// Write a file under `root`, creating parent directories on demand.
fn write_file(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
}

/// Capture a real-tool run without an unbounded wait or full output pipes.
pub(crate) fn bounded_output(command: &mut Command) -> std::io::Result<Output> {
    use std::io::{Read, Seek};
    use std::time::{Duration, Instant};
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let mut child = command
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?)
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= Duration::from_secs(60) {
            child.kill()?;
            let _status = child.wait()?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Lockpick fixture exceeded 60 seconds",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    stdout.rewind()?;
    stderr.rewind()?;
    let mut out = Output {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let _stdout_len = stdout.read_to_end(&mut out.stdout)?;
    let _stderr_len = stderr.read_to_end(&mut out.stderr)?;
    Ok(out)
}
