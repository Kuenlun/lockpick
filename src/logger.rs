/*!
lockpick - Rust CLI to enforce merge checks and code quality
Copyright (C) 2026  Juan Luis Leal Contreras (Kuenlun)

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

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
