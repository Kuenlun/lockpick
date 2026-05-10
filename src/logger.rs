// SPDX-License-Identifier: MIT OR Apache-2.0
// lockpick - Rust CLI to enforce merge checks and code quality
// Copyright (c) 2026 Juan Luis Leal Contreras (Kuenlun)

use colored::Colorize;
use indicatif::MultiProgress;
use log::{Level, LevelFilter, Log, Metadata, Record};

struct MultiProgressLogger {
    mp: MultiProgress,
    level: LevelFilter,
    is_tty: bool,
}

impl Log for MultiProgressLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level_tag = match record.level() {
                Level::Info => format!("[{}]", record.level()).green().to_string(),
                _ => format!("[{}]", record.level()),
            };
            let msg = format!("{level_tag} {}", record.args());
            if self.is_tty {
                let _ = self.mp.println(msg);
            } else {
                eprintln!("{msg}");
            }
        }
    }

    fn flush(&self) {}
}

pub fn init(verbosity: u8, mp: &MultiProgress, is_tty: bool) {
    let level = match verbosity {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let logger = MultiProgressLogger {
        mp: mp.clone(),
        level,
        is_tty,
    };

    let _ = log::set_boxed_logger(Box::new(logger));
    log::set_max_level(level);
}
