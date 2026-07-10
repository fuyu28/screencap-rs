//! PNG encoding through WIC, 32bpp BGRA.

use windows::core::{Error as WinError, HRESULT, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, DeleteFileW, GetFileAttributesW, GetFinalPathNameByHandleW,
    FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_NAME_NORMALIZED, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};

use crate::types::{ErrorInfo, ImageBuffer};
use crate::util::wide_from_utf8;

const WHERE: &str = "SavePngWic";

/// Calls `CoUninitialize` on drop only if this call to `CoInitializeEx`
/// actually initialized COM.
struct CoInitGuard {
    active: bool,
}

impl Drop for CoInitGuard {
    fn drop(&mut self) {
        if self.active {
            unsafe { CoUninitialize() };
        }
    }
}

/// Normalizes forward slashes to backslashes. The Win32 file APIs accept
/// either separator, but WIC's shell-based stream creation is fussier and the
/// resolved path is reported back to the user, so we settle on one separator
/// up front. `/` is a valid separator on Windows, so this must never reject
/// input -- it just rewrites it.
pub fn normalize_path_separators(path: &str) -> String {
    path.replace('/', "\\")
}

/// Strips the `\\?\` (or `\\?\UNC\`) extended-length prefix that
/// `GetFinalPathNameByHandleW` returns, so a reported path matches what a user
/// would recognise.
fn strip_extended_length_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

/// Resolves the real on-disk path of `requested`, including the actual casing
/// the filesystem stored. Windows volumes are case-insensitive, so asking to
/// write `test.png` when `TEST.png` already exists truncates and reuses the
/// existing `TEST.png`; this reports that true name so a success message never
/// points at a `test.png` that was never created. Falls back to the
/// separator-normalized `requested` when the file cannot be opened (e.g. it
/// does not exist).
pub fn real_output_path(requested: &str) -> String {
    let fallback = || normalize_path_separators(requested);

    let mut wide = wide_from_utf8(requested);
    wide.push(0);

    let handle = unsafe {
        CreateFileW(
            PCWSTR::from_raw(wide.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };
    let handle = match handle {
        Ok(h) if !h.is_invalid() => h,
        _ => return fallback(),
    };

    struct HandleGuard(HANDLE);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let guard = HandleGuard(handle);

    // GetFinalPathNameByHandleW returns the length without the NUL when the
    // buffer fits, or the required length *with* the NUL when it does not.
    let mut buf = vec![0u16; 512];
    for _ in 0..3 {
        let len = unsafe { GetFinalPathNameByHandleW(guard.0, &mut buf, FILE_NAME_NORMALIZED) };
        if len == 0 {
            break;
        }
        let len = len as usize;
        if len <= buf.len() {
            let real = String::from_utf16_lossy(&buf[..len]);
            return strip_extended_length_prefix(&real);
        }
        buf = vec![0u16; len];
    }
    fallback()
}

/// Returns the parent directory that must already exist for `normalized` (an
/// already-separator-normalized path) to be writable, or `None` when the path
/// is a bare filename saved to the current directory (nothing to check).
/// `Path::parent` yields `Some("")` for such filenames, which is treated the
/// same as no parent.
///
/// Callers should normalize separators (see [`normalize_path_separators`])
/// before calling so the parent matches what the capture backend checks.
pub fn output_parent_dir(normalized: &str) -> Option<&str> {
    match std::path::Path::new(normalized)
        .parent()
        .and_then(|p| p.to_str())
    {
        Some("") | None => None,
        Some(parent) => Some(parent),
    }
}

fn hr_error(message: &str, hr: HRESULT) -> ErrorInfo {
    ErrorInfo::with_hresult(message, WHERE, hr.0 as u32)
}

fn win_error(message: &str, e: WinError) -> ErrorInfo {
    ErrorInfo::with_hresult(message, WHERE, e.code().0 as u32)
}

/// Refuses to overwrite an existing file unless `overwrite`
/// ("output exists (use --overwrite)"). Handles COM init/uninit internally
/// (tolerates RPC_E_CHANGED_MODE).
pub fn save_png_wic(img: &ImageBuffer, out_path: &str, overwrite: bool) -> Result<(), ErrorInfo> {
    // `/` is a valid separator on Windows; normalize it so WIC's
    // InitializeFromFilename (shell-based) reliably accepts the path instead of
    // failing opaquely.
    let normalized = normalize_path_separators(out_path);
    let mut wide_path = wide_from_utf8(&normalized);
    wide_path.push(0);
    let wide_path = PCWSTR::from_raw(wide_path.as_ptr());

    if !overwrite {
        let attrs = unsafe { GetFileAttributesW(wide_path) };
        if attrs != INVALID_FILE_ATTRIBUTES {
            return Err(ErrorInfo::new("output exists (use --overwrite)", WHERE));
        }
    }

    // WIC's InitializeFromFilename fails opaquely (ERROR_PATH_NOT_FOUND) when
    // the parent directory is missing; check it up front and report a clear,
    // specific error rather than creating the directory ourselves.
    if let Some(parent) = output_parent_dir(&normalized) {
        let mut wide_parent = wide_from_utf8(parent);
        wide_parent.push(0);
        let attrs = unsafe { GetFileAttributesW(PCWSTR::from_raw(wide_parent.as_ptr())) };
        if attrs == INVALID_FILE_ATTRIBUTES || (attrs & FILE_ATTRIBUTE_DIRECTORY.0) == 0 {
            return Err(ErrorInfo::new(
                format!("output directory does not exist: {parent}"),
                WHERE,
            ));
        }
    }

    let mut hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut need_uninit = hr.is_ok();
    if hr == RPC_E_CHANGED_MODE {
        need_uninit = false;
        hr = HRESULT(0);
    }
    if hr.is_err() {
        return Err(hr_error("CoInitializeEx failed", hr));
    }
    let _co_guard = CoInitGuard {
        active: need_uninit,
    };

    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| win_error("CoCreateInstance IWICImagingFactory failed", e))?;

    let stream =
        unsafe { factory.CreateStream() }.map_err(|e| win_error("CreateStream failed", e))?;

    unsafe { stream.InitializeFromFilename(wide_path, GENERIC_WRITE.0) }
        .map_err(|e| win_error("InitializeFromFilename failed", e))?;

    // InitializeFromFilename already created/truncated the output file. If
    // any later step fails, delete the partial file instead of leaving a
    // 0-byte (or corrupt) file behind that would trip the overwrite guard on
    // retry.
    let encode = || -> Result<(), ErrorInfo> {
        let encoder = unsafe { factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()) }
            .map_err(|e| win_error("CreateEncoder failed", e))?;

        unsafe { encoder.Initialize(&stream, WICBitmapEncoderNoCache) }
            .map_err(|e| win_error("Encoder Initialize failed", e))?;

        let mut frame = None;
        let mut props = None;
        unsafe { encoder.CreateNewFrame(&mut frame, &mut props) }
            .map_err(|e| win_error("CreateNewFrame failed", e))?;
        let frame = frame.ok_or_else(|| ErrorInfo::new("CreateNewFrame failed", WHERE))?;

        unsafe { frame.Initialize(props.as_ref()) }
            .map_err(|e| win_error("Frame Initialize failed", e))?;

        unsafe { frame.SetSize(img.width as u32, img.height as u32) }
            .map_err(|e| win_error("SetSize failed", e))?;

        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        unsafe { frame.SetPixelFormat(&mut fmt) }
            .map_err(|e| win_error("SetPixelFormat failed", e))?;

        unsafe { frame.WritePixels(img.height as u32, img.row_pitch as u32, &img.bgra) }
            .map_err(|e| win_error("WritePixels failed", e))?;

        unsafe { frame.Commit() }.map_err(|e| win_error("Frame Commit failed", e))?;
        unsafe { encoder.Commit() }.map_err(|e| win_error("Encoder Commit failed", e))?;

        Ok(())
    };

    let result = encode();
    if let Err(e) = result {
        // The stream still holds the file open with GENERIC_WRITE (and no
        // FILE_SHARE_DELETE), so it must be released before DeleteFileW can
        // succeed.
        drop(stream);
        unsafe {
            let _ = DeleteFileW(wide_path);
        }
        return Err(e);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rewrites_forward_slashes() {
        assert_eq!(normalize_path_separators("a/b/c.png"), r"a\b\c.png");
        assert_eq!(
            normalize_path_separators("C:/Users/me/shot.png"),
            r"C:\Users\me\shot.png"
        );
    }

    #[test]
    fn normalize_leaves_backslash_paths_untouched() {
        assert_eq!(normalize_path_separators(r"C:\dir\x.png"), r"C:\dir\x.png");
        assert_eq!(normalize_path_separators("plain.png"), "plain.png");
        assert_eq!(normalize_path_separators(""), "");
    }

    #[test]
    fn output_parent_dir_skips_bare_filenames() {
        assert_eq!(output_parent_dir("plain.png"), None);
        assert_eq!(output_parent_dir(""), None);
    }

    #[test]
    fn output_parent_dir_returns_directory_component() {
        assert_eq!(output_parent_dir(r"a\b.png"), Some("a"));
        assert_eq!(output_parent_dir(r"C:\dir\x.png"), Some(r"C:\dir"));
    }

    /// Minimal opaque BGRA buffer for exercising the WIC save path.
    #[cfg(windows)]
    fn tiny_image() -> ImageBuffer {
        let width = 2;
        let height = 2;
        ImageBuffer {
            width,
            height,
            row_pitch: width * 4,
            origin_x: 0,
            origin_y: 0,
            bgra: vec![255u8; (width * height * 4) as usize],
        }
    }

    #[cfg(windows)]
    #[test]
    fn save_png_wic_reports_missing_output_directory() {
        let img = tiny_image();
        let err = save_png_wic(&img, "definitely-missing-dir-xyz/out.png", true).unwrap_err();
        assert!(
            err.message.contains("output directory does not exist"),
            "unexpected error: {err:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn save_png_wic_writes_file_to_existing_directory() {
        let mut path = std::env::temp_dir();
        path.push(format!("screencap_savepng_{}.png", std::process::id()));
        let path_str = path.to_string_lossy().into_owned();

        save_png_wic(&tiny_image(), &path_str, true).expect("save should succeed");
        assert!(path.exists(), "expected {path:?} to exist");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn strip_prefix_handles_dos_and_unc_and_plain() {
        assert_eq!(
            strip_extended_length_prefix(r"\\?\C:\dir\TEST.png"),
            r"C:\dir\TEST.png"
        );
        assert_eq!(
            strip_extended_length_prefix(r"\\?\UNC\server\share\a.png"),
            r"\\server\share\a.png"
        );
        assert_eq!(
            strip_extended_length_prefix(r"C:\already\clean.png"),
            r"C:\already\clean.png"
        );
    }
}
