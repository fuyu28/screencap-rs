//! DXGI Output Duplication capture.
//! Methods: dxgi-monitor, dxgi-window (monitor resolved from ctx.monitor or
//! the window's nearest monitor).

use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};

use crate::d3d11_copy::{copy_texture_to_image, create_d3d11_device};
use crate::types::{CaptureContext, ErrorInfo, ImageBuffer, Rect};

fn hr_error(message: &str, where_: &str, e: &windows::core::Error) -> ErrorInfo {
    ErrorInfo::with_hresult(message, where_, e.code().0 as u32)
}

struct DupFrameGuard {
    dup: IDXGIOutputDuplication,
}

impl Drop for DupFrameGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = self.dup.ReleaseFrame();
        }
    }
}

fn find_output_for_monitor(
    hmon: HMONITOR,
) -> Result<(IDXGIAdapter1, IDXGIOutput1, i32, i32), ErrorInfo> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|e| hr_error("CreateDXGIFactory1 failed", "FindOutputForMonitor", &e))?;

    let mut ai: u32 = 0;
    loop {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(ai) } {
            Ok(adapter) => adapter,
            Err(_) => break,
        };

        let mut oi: u32 = 0;
        loop {
            let output = match unsafe { adapter.EnumOutputs(oi) } {
                Ok(output) => output,
                Err(_) => break,
            };

            let desc = unsafe { output.GetDesc() }
                .map_err(|e| hr_error("GetDesc failed", "FindOutputForMonitor", &e))?;
            if desc.Monitor == hmon {
                let output1: IDXGIOutput1 = output.cast().map_err(|e| {
                    hr_error(
                        "QueryInterface IDXGIOutput1 failed",
                        "FindOutputForMonitor",
                        &e,
                    )
                })?;
                return Ok((adapter, output1, ai as i32, oi as i32));
            }

            oi += 1;
        }

        ai += 1;
    }

    Err(ErrorInfo::new(
        "monitor output not found",
        "FindOutputForMonitor",
    ))
}

fn acquire_dup_frame(
    output1: &IDXGIOutput1,
    adapter: &IDXGIAdapter1,
    timeout_ms: i32,
    capture_rect: Rect,
) -> Result<ImageBuffer, ErrorInfo> {
    let (device, context) =
        create_d3d11_device(Some(adapter), D3D_DRIVER_TYPE_UNKNOWN, "AcquireDupFrame")?;

    let dup: IDXGIOutputDuplication = unsafe { output1.DuplicateOutput(&device) }
        .map_err(|e| hr_error("DuplicateOutput failed", "AcquireDupFrame", &e))?;

    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;
    unsafe { dup.AcquireNextFrame(timeout_ms as u32, &mut frame_info, &mut resource) }
        .map_err(|e| hr_error("AcquireNextFrame failed", "AcquireDupFrame", &e))?;
    let _frame = DupFrameGuard { dup };
    let resource = resource.expect("AcquireNextFrame succeeded without a resource");

    let tex: ID3D11Texture2D = resource
        .cast()
        .map_err(|e| hr_error("frame resource to texture failed", "AcquireDupFrame", &e))?;

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { tex.GetDesc(&mut desc) };

    let image = copy_texture_to_image(
        &device,
        &context,
        desc,
        capture_rect,
        |staging| unsafe { context.CopyResource(staging, &tex) },
        "AcquireDupFrame",
    )?;

    Ok(image)
}

/// On success also returns (adapter_index, output_index) for logging.
pub fn capture_with_dxgi(ctx: &CaptureContext) -> Result<(ImageBuffer, i32, i32), ErrorInfo> {
    let hmon: HMONITOR = if let Some(monitor) = &ctx.monitor {
        HMONITOR(monitor.hmon as *mut _)
    } else if let Some(window) = &ctx.window {
        unsafe { MonitorFromWindow(HWND(window.hwnd as *mut _), MONITOR_DEFAULTTONEAREST) }
    } else {
        HMONITOR::default()
    };
    if hmon.0.is_null() {
        return Err(ErrorInfo::new(
            "unable to resolve monitor for DXGI",
            "CaptureWithDxgi",
        ));
    }

    let (adapter, output, ai, oi) = find_output_for_monitor(hmon)?;

    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
        return Err(ErrorInfo::with_win32(
            "GetMonitorInfo failed",
            "CaptureWithDxgi",
            unsafe { windows::Win32::Foundation::GetLastError() }.0,
        ));
    }
    let monitor_rect = Rect::from(mi.rcMonitor);

    let mut full = acquire_dup_frame(&output, &adapter, ctx.common.timeout_ms, monitor_rect)?;

    if ctx.cap.force_alpha_255 {
        for i in (3..full.bgra.len()).step_by(4) {
            full.bgra[i] = 0xFF;
        }
    }

    Ok((full, ai, oi))
}
