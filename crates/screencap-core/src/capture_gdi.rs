//! Port of src/capture_gdi.cpp.
//! Methods: gdi-printwindow, gdi-bitblt-client, gdi-bitblt-windowdc,
//! gdi-bitblt-screen.

use std::ffi::c_void;

use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetWindowDC,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HDC,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::PW_RENDERFULLCONTENT;

use crate::types::{CaptureContext, ErrorInfo, ImageBuffer};

/// Manual FFI binding for `PrintWindow`. In the `windows` crate this function
/// lives under `Win32::Storage::Xps`, gated behind the `Win32_Storage_Xps`
/// feature (which this crate does not enable). Declare it directly using the
/// same `link!` mechanism the crate uses internally, rather than pulling in
/// an extra feature.
unsafe fn print_window(hwnd: HWND, hdc_blt: HDC, flags: u32) -> windows::core::BOOL {
    windows::core::link!("user32.dll" "system" fn PrintWindow(hwnd: HWND, hdcblt: HDC, nflags: u32) -> windows::core::BOOL);
    unsafe { PrintWindow(hwnd, hdc_blt, flags) }
}

/// Top-down, 32bpp, BI_RGB `BITMAPINFO` for a `w`x`h` DIB section.
fn bitmap_info(w: i32, h: i32) -> BITMAPINFO {
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;
    bmi
}

/// Port of the anonymous `CaptureFromDc` helper: create a memory DC + DIB
/// section, `BitBlt` from `src_dc`, and copy the resulting bits into an
/// `ImageBuffer`.
fn capture_from_dc(
    src_dc: HDC,
    src_x: i32,
    src_y: i32,
    w: i32,
    h: i32,
    origin_x: i32,
    origin_y: i32,
) -> Result<ImageBuffer, ErrorInfo> {
    unsafe {
        let mem_dc = CreateCompatibleDC(Some(src_dc));
        if mem_dc.is_invalid() {
            return Err(ErrorInfo::with_win32(
                "CreateCompatibleDC failed",
                "CaptureFromDc",
                GetLastError().0,
            ));
        }

        let bmi = bitmap_info(w, h);
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        let bmp = match dib {
            Ok(b) if !b.is_invalid() && !bits.is_null() => b,
            other => {
                let code = GetLastError().0;
                if let Ok(b) = other {
                    let _ = DeleteObject(b.into());
                }
                let _ = DeleteDC(mem_dc);
                return Err(ErrorInfo::with_win32(
                    "CreateDIBSection failed",
                    "CaptureFromDc",
                    code,
                ));
            }
        };

        let old = SelectObject(mem_dc, bmp.into());
        let blt = BitBlt(mem_dc, 0, 0, w, h, Some(src_dc), src_x, src_y, SRCCOPY | CAPTUREBLT);
        if blt.is_err() {
            let code = GetLastError().0;
            let _ = SelectObject(mem_dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem_dc);
            return Err(ErrorInfo::with_win32("BitBlt failed", "CaptureFromDc", code));
        }

        let row_pitch = w * 4;
        let len = row_pitch as usize * h as usize;
        let bgra = std::slice::from_raw_parts(bits as *const u8, len).to_vec();

        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem_dc);

        Ok(ImageBuffer {
            width: w,
            height: h,
            row_pitch,
            origin_x,
            origin_y,
            bgra,
        })
    }
}

pub fn capture_with_gdi(ctx: &CaptureContext) -> Result<ImageBuffer, ErrorInfo> {
    match ctx.method.as_str() {
        "gdi-printwindow" => {
            let w = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new("gdi-printwindow requires window target", "CaptureWithGdi")
            })?;
            let width = w.rect.width();
            let height = w.rect.height();
            let hwnd = HWND(w.hwnd as *mut _);

            unsafe {
                let win_dc = GetWindowDC(Some(hwnd));
                if win_dc.is_invalid() {
                    return Err(ErrorInfo::with_win32(
                        "GetWindowDC failed",
                        "CaptureWithGdi",
                        GetLastError().0,
                    ));
                }

                let mem_dc = CreateCompatibleDC(Some(win_dc));
                let bmi = bitmap_info(width, height);
                let mut bits: *mut c_void = std::ptr::null_mut();
                let dib = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
                let bmp = match dib {
                    Ok(b) if !b.is_invalid() && !bits.is_null() => b,
                    other => {
                        let code = GetLastError().0;
                        if let Ok(b) = other {
                            let _ = DeleteObject(b.into());
                        }
                        let _ = DeleteDC(mem_dc);
                        ReleaseDC(Some(hwnd), win_dc);
                        return Err(ErrorInfo::with_win32(
                            "CreateDIBSection failed",
                            "CaptureWithGdi",
                            code,
                        ));
                    }
                };

                let old = SelectObject(mem_dc, bmp.into());
                let ok = print_window(hwnd, mem_dc, PW_RENDERFULLCONTENT);
                if !ok.as_bool() {
                    let code = GetLastError().0;
                    let _ = SelectObject(mem_dc, old);
                    let _ = DeleteObject(bmp.into());
                    let _ = DeleteDC(mem_dc);
                    ReleaseDC(Some(hwnd), win_dc);
                    return Err(ErrorInfo::with_win32(
                        "PrintWindow failed",
                        "CaptureWithGdi",
                        code,
                    ));
                }

                let row_pitch = width * 4;
                let len = row_pitch as usize * height as usize;
                let bgra = std::slice::from_raw_parts(bits as *const u8, len).to_vec();

                let _ = SelectObject(mem_dc, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem_dc);
                ReleaseDC(Some(hwnd), win_dc);

                Ok(ImageBuffer {
                    width,
                    height,
                    row_pitch,
                    origin_x: w.rect.left,
                    origin_y: w.rect.top,
                    bgra,
                })
            }
        }

        "gdi-bitblt-client" => {
            let w = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new("gdi-bitblt-client requires window target", "CaptureWithGdi")
            })?;
            let hwnd = HWND(w.hwnd as *mut _);
            let src = unsafe { GetDC(Some(hwnd)) };
            if src.is_invalid() {
                return Err(ErrorInfo::with_win32(
                    "GetDC(hwnd) failed",
                    "CaptureWithGdi",
                    unsafe { GetLastError().0 },
                ));
            }
            let ww = w.client_rect_screen.width();
            let hh = w.client_rect_screen.height();
            let result = capture_from_dc(
                src,
                0,
                0,
                ww,
                hh,
                w.client_rect_screen.left,
                w.client_rect_screen.top,
            );
            unsafe { ReleaseDC(Some(hwnd), src) };
            result
        }

        "gdi-bitblt-windowdc" => {
            let w = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new("gdi-bitblt-windowdc requires window target", "CaptureWithGdi")
            })?;
            let hwnd = HWND(w.hwnd as *mut _);
            let src = unsafe { GetWindowDC(Some(hwnd)) };
            if src.is_invalid() {
                return Err(ErrorInfo::with_win32(
                    "GetWindowDC failed",
                    "CaptureWithGdi",
                    unsafe { GetLastError().0 },
                ));
            }
            let ww = w.rect.width();
            let hh = w.rect.height();
            let result = capture_from_dc(src, 0, 0, ww, hh, w.rect.left, w.rect.top);
            unsafe { ReleaseDC(Some(hwnd), src) };
            result
        }

        "gdi-bitblt-screen" => {
            let src = unsafe { GetDC(None) };
            if src.is_invalid() {
                return Err(ErrorInfo::with_win32(
                    "GetDC(NULL) failed",
                    "CaptureWithGdi",
                    unsafe { GetLastError().0 },
                ));
            }
            let r = ctx.capture_rect_screen;
            let ww = r.width();
            let hh = r.height();
            let result = capture_from_dc(src, r.left, r.top, ww, hh, r.left, r.top);
            unsafe { ReleaseDC(None, src) };
            result
        }

        _ => Err(ErrorInfo::new("unknown gdi method", "CaptureWithGdi")),
    }
}
