use crate::types::LogLevel;
use crate::util::{build_timestamp_for_filename, iso8601_now_local};
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{s, w};

struct LoggerState {
    file: Option<File>,
    file_path: PathBuf,
}

/// Thread-safe file logger. Log lines: `[<iso8601>] [<level>] <msg>\n`,
/// written per line. File name: `<timestamp>_<pid>_<command>.log` in log_dir.
pub struct Logger {
    inner: Mutex<LoggerState>,
    // Kept outside the mutex so log() can reject below-threshold messages
    // without locking.
    min_level: AtomicU8,
}

impl Default for Logger {
    fn default() -> Self {
        Self {
            inner: Mutex::new(LoggerState {
                file: None,
                file_path: PathBuf::new(),
            }),
            min_level: AtomicU8::new(LogLevel::Info as u8),
        }
    }
}

impl Logger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(
        &mut self,
        log_dir_utf8: &str,
        command_name: &str,
        level: LogLevel,
    ) -> io::Result<()> {
        self.min_level.store(level as u8, Ordering::Relaxed);

        let mut state = self.inner.lock().unwrap();

        let dir = PathBuf::from(log_dir_utf8);
        fs::create_dir_all(&dir)?;

        let base_name = if command_name.is_empty() {
            "unknown"
        } else {
            command_name
        };
        let filename = format!(
            "{}_{}_{}.log",
            build_timestamp_for_filename(),
            std::process::id(),
            base_name
        );
        let file_path = dir.join(filename);

        let file = File::create(&file_path)?;
        state.file_path = file_path;
        state.file = Some(file);
        Ok(())
    }

    pub fn log(&self, level: LogLevel, msg: &str) {
        if (level as u8) < self.min_level.load(Ordering::Relaxed) {
            return;
        }
        let line = format!(
            "[{}] [{}] {}\n",
            iso8601_now_local(),
            log_level_name(level),
            msg
        );

        let mut state = self.inner.lock().unwrap();
        let Some(file) = state.file.as_mut() else {
            return;
        };
        let _ = file.write_all(line.as_bytes());
    }

    pub fn file_path(&self) -> PathBuf {
        self.inner.lock().unwrap().file_path.clone()
    }
}

/// trace/debug/warn/error, anything else -> Info.
pub fn parse_log_level(s: &str) -> LogLevel {
    match s {
        "trace" => LogLevel::Trace,
        "debug" => LogLevel::Debug,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

pub fn log_level_name(lv: LogLevel) -> &'static str {
    match lv {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

pub fn get_build_stamp() -> String {
    "rust build".to_string()
}

#[repr(C)]
struct RtlOsVersionInfoW {
    dw_os_version_info_size: u32,
    dw_major_version: u32,
    dw_minor_version: u32,
    dw_build_number: u32,
    dw_platform_id: u32,
    sz_csd_version: [u16; 128],
}

type RtlGetVersionFn = unsafe extern "system" fn(*mut RtlOsVersionInfoW) -> i32;

/// `Windows <major>.<minor> build <build>` via RtlGetVersion, or "unknown".
pub fn get_os_version_string() -> String {
    unsafe {
        let ntdll = match GetModuleHandleW(w!("ntdll.dll")) {
            Ok(h) => h,
            Err(_) => return "unknown".to_string(),
        };
        let proc = GetProcAddress(ntdll, s!("RtlGetVersion"));
        let func = match proc {
            Some(f) => f,
            None => return "unknown".to_string(),
        };
        let func: RtlGetVersionFn = std::mem::transmute(func);

        let mut osv = RtlOsVersionInfoW {
            dw_os_version_info_size: std::mem::size_of::<RtlOsVersionInfoW>() as u32,
            dw_major_version: 0,
            dw_minor_version: 0,
            dw_build_number: 0,
            dw_platform_id: 0,
            sz_csd_version: [0u16; 128],
        };
        let status = func(&mut osv);
        if status != 0 {
            return "unknown".to_string();
        }
        format!(
            "Windows {}.{} build {}",
            osv.dw_major_version, osv.dw_minor_version, osv.dw_build_number
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    #[test]
    fn parse_log_level_maps_known_strings() {
        assert_eq!(parse_log_level("trace"), LogLevel::Trace);
        assert_eq!(parse_log_level("debug"), LogLevel::Debug);
        assert_eq!(parse_log_level("warn"), LogLevel::Warn);
        assert_eq!(parse_log_level("error"), LogLevel::Error);
    }

    #[test]
    fn parse_log_level_defaults_to_info() {
        assert_eq!(parse_log_level("info"), LogLevel::Info);
        assert_eq!(parse_log_level("bogus"), LogLevel::Info);
        assert_eq!(parse_log_level(""), LogLevel::Info);
        // Case-sensitive: uppercase is not recognised and falls back to Info.
        assert_eq!(parse_log_level("TRACE"), LogLevel::Info);
    }

    #[test]
    fn log_level_names_round_trip_through_parse() {
        for lv in [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ] {
            assert_eq!(parse_log_level(log_level_name(lv)), lv);
        }
    }

    #[test]
    fn log_levels_are_ordered_by_severity() {
        assert!(LogLevel::Trace < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn logger_filters_below_min_level() {
        let dir = std::env::temp_dir().join(format!(
            "screencap_log_test_{}_{}",
            std::process::id(),
            build_timestamp_for_filename()
        ));
        let mut logger = Logger::new();
        logger
            .init(dir.to_str().unwrap(), "unittest", LogLevel::Warn)
            .expect("logger init should succeed");

        logger.log(LogLevel::Info, "below-threshold message");
        logger.log(LogLevel::Error, "kept message");

        let mut contents = String::new();
        File::open(logger.file_path())
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        assert!(!contents.contains("below-threshold message"));
        assert!(contents.contains("kept message"));
        assert!(contents.contains("[error]"));

        let _ = fs::remove_dir_all(&dir);
    }
}
