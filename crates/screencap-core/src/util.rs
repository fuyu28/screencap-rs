//! Port of src/util.cpp and the inline helpers in src/common.h.

use chrono::Local;

/// JSON string escaping identical to the C++ JsonEscape.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Local time as `2026-07-08T12:34:56.789+09:00`.
pub fn iso8601_now_local() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
}

/// Local time as `20260708_123456_789` for log/screenshot filenames.
pub fn build_timestamp_for_filename() -> String {
    Local::now().format("%Y%m%d_%H%M%S_%3f").to_string()
}

/// `0x%08X` formatting for HRESULTs.
pub fn to_hex32(v: u32) -> String {
    format!("0x{:08X}", v)
}

/// UTF-8 -> UTF-16 (no trailing NUL).
pub fn wide_from_utf8(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// UTF-16 slice (no trailing NUL) -> UTF-8, lossy.
pub fn utf8_from_wide(ws: &[u16]) -> String {
    String::from_utf16_lossy(ws)
}
