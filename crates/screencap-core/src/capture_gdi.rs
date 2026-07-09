//! Methods: gdi-printwindow, gdi-bitblt-client, gdi-bitblt-windowdc,
//! gdi-bitblt-screen.

use std::ffi::c_void;

use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetWindowDC,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ, SRCCOPY,
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

struct DeviceContext {
    owner: Option<HWND>,
    hdc: HDC,
}

impl DeviceContext {
    fn window(hwnd: HWND) -> Result<Self, ErrorInfo> {
        let hdc = unsafe { GetWindowDC(Some(hwnd)) };
        Self::from_raw(Some(hwnd), hdc, "GetWindowDC failed")
    }

    fn client(hwnd: HWND) -> Result<Self, ErrorInfo> {
        let hdc = unsafe { GetDC(Some(hwnd)) };
        Self::from_raw(Some(hwnd), hdc, "GetDC(hwnd) failed")
    }

    fn screen() -> Result<Self, ErrorInfo> {
        let hdc = unsafe { GetDC(None) };
        Self::from_raw(None, hdc, "GetDC(NULL) failed")
    }

    fn from_raw(owner: Option<HWND>, hdc: HDC, message: &str) -> Result<Self, ErrorInfo> {
        if hdc.is_invalid() {
            Err(ErrorInfo::with_win32(message, "CaptureWithGdi", unsafe {
                GetLastError().0
            }))
        } else {
            Ok(Self { owner, hdc })
        }
    }

    fn get(&self) -> HDC {
        self.hdc
    }
}

impl Drop for DeviceContext {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(self.owner, self.hdc);
        }
    }
}

struct MemoryDc(HDC);

impl MemoryDc {
    fn new(compatible_dc: HDC) -> Result<Self, ErrorInfo> {
        let hdc = unsafe { CreateCompatibleDC(Some(compatible_dc)) };
        if hdc.is_invalid() {
            Err(ErrorInfo::with_win32(
                "CreateCompatibleDC failed",
                "CaptureWithDib",
                unsafe { GetLastError().0 },
            ))
        } else {
            Ok(Self(hdc))
        }
    }

    fn get(&self) -> HDC {
        self.0
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

struct DibSection {
    bitmap: HBITMAP,
    bits: *mut c_void,
}

impl DibSection {
    fn new(dc: HDC, w: i32, h: i32) -> Result<Self, ErrorInfo> {
        let bmi = bitmap_info(w, h);
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = unsafe { CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) };
        match dib {
            Ok(bitmap) if !bitmap.is_invalid() && !bits.is_null() => Ok(Self { bitmap, bits }),
            other => {
                let code = unsafe { GetLastError().0 };
                if let Ok(bitmap) = other {
                    unsafe {
                        let _ = DeleteObject(bitmap.into());
                    }
                }
                Err(ErrorInfo::with_win32(
                    "CreateDIBSection failed",
                    "CaptureWithDib",
                    code,
                ))
            }
        }
    }

    fn bits(&self) -> *mut c_void {
        self.bits
    }

    fn as_object(&self) -> HGDIOBJ {
        self.bitmap.into()
    }
}

impl Drop for DibSection {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.bitmap.into());
        }
    }
}

struct SelectedObject {
    dc: HDC,
    old: HGDIOBJ,
}

impl SelectedObject {
    fn new(dc: HDC, object: HGDIOBJ) -> Self {
        let old = unsafe { SelectObject(dc, object) };
        Self { dc, old }
    }
}

impl Drop for SelectedObject {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.dc, self.old);
        }
    }
}

fn capture_with_dib(
    compatible_dc: HDC,
    w: i32,
    h: i32,
    origin_x: i32,
    origin_y: i32,
    draw: impl FnOnce(HDC) -> Result<(), ErrorInfo>,
) -> Result<ImageBuffer, ErrorInfo> {
    let mem_dc = MemoryDc::new(compatible_dc)?;
    let dib = DibSection::new(mem_dc.get(), w, h)?;
    let _selected = SelectedObject::new(mem_dc.get(), dib.as_object());

    draw(mem_dc.get())?;

    let row_pitch = w * 4;
    let len = row_pitch as usize * h as usize;
    let bgra = unsafe { std::slice::from_raw_parts(dib.bits() as *const u8, len).to_vec() };

    Ok(ImageBuffer {
        width: w,
        height: h,
        row_pitch,
        origin_x,
        origin_y,
        bgra,
    })
}

/// Capture from a device context by `BitBlt`-ing into a 32bpp top-down DIB.
fn capture_from_dc(
    src_dc: HDC,
    src_x: i32,
    src_y: i32,
    w: i32,
    h: i32,
    origin_x: i32,
    origin_y: i32,
) -> Result<ImageBuffer, ErrorInfo> {
    capture_with_dib(src_dc, w, h, origin_x, origin_y, |mem_dc| {
        let blt = unsafe {
            BitBlt(
                mem_dc,
                0,
                0,
                w,
                h,
                Some(src_dc),
                src_x,
                src_y,
                SRCCOPY | CAPTUREBLT,
            )
        };
        if blt.is_err() {
            Err(ErrorInfo::with_win32(
                "BitBlt failed",
                "CaptureFromDc",
                unsafe { GetLastError().0 },
            ))
        } else {
            Ok(())
        }
    })
}

pub fn capture_with_gdi(ctx: &CaptureContext) -> Result<ImageBuffer, ErrorInfo> {
    match ctx.cap.method.as_str() {
        "gdi-printwindow" => {
            let w = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new("gdi-printwindow requires window target", "CaptureWithGdi")
            })?;
            let width = w.rect.width();
            let height = w.rect.height();
            let hwnd = HWND(w.hwnd as *mut _);

            let win_dc = DeviceContext::window(hwnd)?;
            capture_with_dib(
                win_dc.get(),
                width,
                height,
                w.rect.left,
                w.rect.top,
                |mem_dc| {
                    let ok = unsafe { print_window(hwnd, mem_dc, PW_RENDERFULLCONTENT) };
                    if ok.as_bool() {
                        Ok(())
                    } else {
                        Err(ErrorInfo::with_win32(
                            "PrintWindow failed",
                            "CaptureWithGdi",
                            unsafe { GetLastError().0 },
                        ))
                    }
                },
            )
        }

        "gdi-bitblt-client" => {
            let w = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new("gdi-bitblt-client requires window target", "CaptureWithGdi")
            })?;
            let hwnd = HWND(w.hwnd as *mut _);
            let src = DeviceContext::client(hwnd)?;
            let ww = w.client_rect_screen.width();
            let hh = w.client_rect_screen.height();
            capture_from_dc(
                src.get(),
                0,
                0,
                ww,
                hh,
                w.client_rect_screen.left,
                w.client_rect_screen.top,
            )
        }

        "gdi-bitblt-windowdc" => {
            let w = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new(
                    "gdi-bitblt-windowdc requires window target",
                    "CaptureWithGdi",
                )
            })?;
            let hwnd = HWND(w.hwnd as *mut _);
            let src = DeviceContext::window(hwnd)?;
            let ww = w.rect.width();
            let hh = w.rect.height();
            capture_from_dc(src.get(), 0, 0, ww, hh, w.rect.left, w.rect.top)
        }

        "gdi-bitblt-screen" => {
            let src = DeviceContext::screen()?;
            let r = ctx.capture_rect_screen;
            let ww = r.width();
            let hh = r.height();
            capture_from_dc(src.get(), r.left, r.top, ww, hh, r.left, r.top)
        }

        _ => Err(ErrorInfo::new("unknown gdi method", "CaptureWithGdi")),
    }
}
