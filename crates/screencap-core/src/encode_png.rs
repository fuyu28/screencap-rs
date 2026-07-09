//! PNG encoding through WIC, 32bpp BGRA.

use windows::core::{Error as WinError, HRESULT, PCWSTR};
use windows::Win32::Foundation::{GENERIC_WRITE, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::Storage::FileSystem::{
    DeleteFileW, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
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
    let mut wide_path = wide_from_utf8(out_path);
    wide_path.push(0);
    let wide_path = PCWSTR::from_raw(wide_path.as_ptr());

    if !overwrite {
        let attrs = unsafe { GetFileAttributesW(wide_path) };
        if attrs != INVALID_FILE_ATTRIBUTES {
            return Err(ErrorInfo::new("output exists (use --overwrite)", WHERE));
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
