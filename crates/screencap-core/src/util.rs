use chrono::Local;

/// Local time as `2026-07-08T12:34:56.789+09:00`.
pub fn iso8601_now_local() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
}

/// Local time as `20260708_123456_789` for log/screenshot filenames.
pub fn build_timestamp_for_filename() -> String {
    Local::now().format("%Y%m%d_%H%M%S_%3f").to_string()
}

/// UTF-8 -> UTF-16 (no trailing NUL).
pub fn wide_from_utf8(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// UTF-16 slice (no trailing NUL) -> UTF-8, lossy.
pub fn utf8_from_wide(ws: &[u16]) -> String {
    String::from_utf16_lossy(ws)
}

/// Characters Windows forbids in a path (excluding the `:` drive separator and
/// the `\`/`/` path separators, which are handled elsewhere).
const INVALID_PATH_CHARS: [char; 6] = ['<', '>', '"', '|', '?', '*'];

/// Validates a user-entered output path before it is handed to the capture
/// backend, returning a human-readable reason on rejection. `/` and `\` are
/// accepted (both are valid separators on Windows); this guards against an
/// empty path, characters the filesystem cannot store, or a path with no
/// file-name component, so bad input surfaces as a clear message instead of an
/// opaque write failure.
///
/// The path is validated as given (un-normalized): `Path::file_name` uses the
/// target's separator rules, so on Windows both `/` and `\` are recognized and
/// a bare root (`C:\`, `\`), a trailing separator, or a `..` ending are all
/// rejected as having no file name.
pub fn validate_output_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("Output path is empty.".to_string());
    }
    for ch in path.chars() {
        if (ch as u32) < 0x20 {
            return Err("Output path contains a control character.".to_string());
        }
        if INVALID_PATH_CHARS.contains(&ch) {
            return Err(format!("Output path contains an invalid character: {ch}"));
        }
    }
    // Do not rely on Path::file_name alone: it strips trailing separators and
    // would accept directory paths such as `C:\dir\`.
    if path.ends_with('/')
        || path.ends_with('\\')
        || std::path::Path::new(path).file_name().is_none()
    {
        return Err(format!("Output path has no file name: {path}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_paths_pass() {
        assert!(validate_output_path(r"C:\Users\me\shot.png").is_ok());
        assert!(validate_output_path("C:/Users/me/shot.png").is_ok());
        assert!(validate_output_path("shot.png").is_ok());
    }

    #[test]
    fn empty_or_blank_paths_are_rejected() {
        assert!(validate_output_path("").is_err());
        assert!(validate_output_path("   ").is_err());
    }

    #[test]
    fn invalid_characters_are_rejected() {
        for bad in [
            "a<b.png", "a>b.png", "a\"b.png", "a|b.png", "a?b.png", "a*b.png",
        ] {
            assert!(
                validate_output_path(bad).is_err(),
                "{bad} should be rejected"
            );
        }
        assert!(validate_output_path("a\u{0007}b.png").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn paths_without_a_file_name_are_rejected() {
        assert!(validate_output_path(r"C:\").is_err());
        assert!(validate_output_path(r"\").is_err());
        assert!(validate_output_path(r"C:\dir\").is_err());
        assert!(validate_output_path(r"a\..").is_err());
    }
}
