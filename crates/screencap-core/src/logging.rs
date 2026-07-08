//! Port of src/logging.cpp / logging.h.

use crate::types::LogLevel;
use std::path::PathBuf;

/// Thread-safe file logger. Log lines: `[<iso8601>] [<level>] <msg>\n`,
/// flushed per line. File name: `<timestamp>_<pid>_<command>.log` in log_dir.
#[derive(Default)]
pub struct Logger {
    // implementation-defined; keep it Sync (interior Mutex).
}

impl Logger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates log_dir and opens the log file. Returns false on failure
    /// (logging then becomes a no-op, matching the C++ behavior).
    pub fn init(&mut self, _log_dir_utf8: &str, _command_name: &str, _level: LogLevel) -> bool {
        todo!("port Logger::Init")
    }

    pub fn log(&self, _level: LogLevel, _msg: &str) {
        todo!("port Logger::Log")
    }

    pub fn file_path(&self) -> PathBuf {
        todo!()
    }
}

/// trace/debug/warn/error, anything else -> Info.
pub fn parse_log_level(_s: &str) -> LogLevel {
    todo!("port ParseLogLevel")
}

pub fn log_level_name(_lv: LogLevel) -> &'static str {
    todo!("port LogLevelName")
}

/// Build date+time stamp (compile-time in C++; a static string is fine here).
pub fn get_build_stamp() -> String {
    todo!("port GetBuildStamp")
}

/// `Windows <major>.<minor> build <build>` via RtlGetVersion, or "unknown".
pub fn get_os_version_string() -> String {
    todo!("port GetOsVersionString")
}
