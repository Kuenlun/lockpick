// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Run every Rust quality gate in one command
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

//! Controlled Cargo and compiler responses for subprocess contract tests.

use std::io::Write;
use std::process::ExitCode;

fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let compiler = std::env::current_exe()?
        .file_stem()
        .is_some_and(|name| name == "rustc");
    if compiler {
        if std::env::var_os("LOCKPICK_TEST_REMOVE_CARGO").is_some() {
            std::fs::remove_file(
                std::env::current_exe()?
                    .with_file_name(format!("cargo{}", std::env::consts::EXE_SUFFIX)),
            )?;
        }
        if args.first().is_some_and(|arg| arg == "--version") {
            println!("rustc {}", std::env::var("LOCKPICK_TEST_CHANNEL")?);
        } else {
            println!("{}", std::env::var("LOCKPICK_TEST_HOST")?);
            if std::env::var("LOCKPICK_TEST_FAIL_HOST").is_ok_and(|value| value == "1") {
                eprintln!("compiler probe failed");
                return Ok(ExitCode::FAILURE);
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let sub = args.first().ok_or("missing Cargo subcommand")?;
    let mut environment: Vec<_> = std::env::vars()
        .filter(|(key, _)| key.starts_with("CARGO_") || key == "CUSTOM_ENV")
        .collect();
    environment.sort();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var_os("LOCKPICK_TEST_LOG").ok_or("missing log path")?)?;
    log.write_all(format!("{args:?} {environment:?}\n").as_bytes())?;
    if sub == "metadata" {
        println!("{}", std::env::var("LOCKPICK_TEST_METADATA")?);
    } else if sub == "llvm-cov" && args.get(1).is_some_and(|arg| arg == "report") {
        println!("{}", std::env::var("LOCKPICK_TEST_REPORT")?);
    } else {
        println!("{sub} stdout");
        eprintln!("{sub} stderr");
    }
    Ok(
        if std::env::var("LOCKPICK_TEST_FAIL").is_ok_and(|fail| fail == *sub) {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        },
    )
}
