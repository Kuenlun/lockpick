use clap::{ArgAction, Parser};
use log::LevelFilter;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Rust merge check CLI to enforce successful build,\
             formatting, Clippy lints, passing tests and code coverage",
    long_about = None
)]
struct Cli {
    #[arg(
        short = 'v',
        long = "verbose",
        action = ArgAction::Count,
        help = "Increase logging verbosity (..= -vvvv)"
    )]
    verbose: u8,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    init_logger(cli.verbose);
    run(cli)
}

fn run(_cli: Cli) -> std::io::Result<()> {
    let common_args = ["--workspace", "--all-targets", "--all-features"];

    log::info!("Running cargo check");
    Command::new("cargo")
        .arg("check")
        .args(common_args)
        .status()?;

    log::info!("Running cargo clippy");
    Command::new("cargo")
        .arg("clippy")
        .args(common_args)
        .status()?;

    log::info!("Running cargo fmt");
    Command::new("cargo").arg("fmt").arg("--check").status()?;

    log::info!("Running cargo test");
    Command::new("cargo")
        .arg("test")
        .args(common_args)
        .status()?;

    Ok(())
}

fn init_logger(verbosity: u8) {
    let level = match verbosity {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    env_logger::Builder::new().filter_level(level).init();
}
