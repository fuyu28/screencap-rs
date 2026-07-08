//! Port of src/util.cpp and the inline helpers in src/common.h.

/// JSON string escaping identical to the C++ JsonEscape.
pub fn json_escape(_s: &str) -> String {
    todo!("port JsonEscape from src/util.cpp")
}

/// Local time as `2026-07-08T12:34:56.789+09:00`.
pub fn iso8601_now_local() -> String {
    todo!("port Iso8601NowLocal from src/util.cpp")
}

/// Local time as `20260708_123456_789` for log/screenshot filenames.
pub fn build_timestamp_for_filename() -> String {
    todo!("port BuildTimestampForFilename from src/util.cpp")
}

/// `0x%08X` formatting for HRESULTs.
pub fn to_hex32(_v: u32) -> String {
    todo!("port ToHex32 from src/common.h")
}

/// UTF-8 -> UTF-16 (no trailing NUL).
pub fn wide_from_utf8(_s: &str) -> Vec<u16> {
    todo!()
}

/// UTF-16 slice (no trailing NUL) -> UTF-8, lossy.
pub fn utf8_from_wide(_ws: &[u16]) -> String {
    todo!()
}
