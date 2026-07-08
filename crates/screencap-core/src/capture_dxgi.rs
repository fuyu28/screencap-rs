//! Port of src/capture_dxgi.cpp (DXGI Output Duplication).
//! Methods: dxgi-monitor, dxgi-window (monitor resolved from ctx.monitor or
//! the window's nearest monitor).

use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};

use crate::types::{CaptureContext, ErrorInfo, ImageBuffer, Rect};

fn hr_error(message: &str, where_: &str, e: &windows::core::Error) -> ErrorInfo {
    ErrorInfo::with_hresult(message, where_, e.code().0 as u32)
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
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|e| hr_error("D3D11CreateDevice failed", "AcquireDupFrame", &e))?;
    let device = device.expect("D3D11CreateDevice succeeded without a device");
    let context = context.expect("D3D11CreateDevice succeeded without a context");

    let dup: IDXGIOutputDuplication = unsafe { output1.DuplicateOutput(&device) }
        .map_err(|e| hr_error("DuplicateOutput failed", "AcquireDupFrame", &e))?;

    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;
    unsafe { dup.AcquireNextFrame(timeout_ms as u32, &mut frame_info, &mut resource) }
        .map_err(|e| hr_error("AcquireNextFrame failed", "AcquireDupFrame", &e))?;
    let resource = resource.expect("AcquireNextFrame succeeded without a resource");

    let tex: ID3D11Texture2D = match resource.cast() {
        Ok(tex) => tex,
        Err(e) => {
            let _ = unsafe { dup.ReleaseFrame() };
            return Err(hr_error(
                "frame resource to texture failed",
                "AcquireDupFrame",
                &e,
            ));
        }
    };

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { tex.GetDesc(&mut desc) };
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    desc.Usage = D3D11_USAGE_STAGING;

    let mut staging: Option<ID3D11Texture2D> = None;
    if let Err(e) = unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) } {
        let _ = unsafe { dup.ReleaseFrame() };
        return Err(hr_error(
            "CreateTexture2D staging failed",
            "AcquireDupFrame",
            &e,
        ));
    }
    let staging = staging.expect("CreateTexture2D succeeded without a texture");

    unsafe { context.CopyResource(&staging, &tex) };

    let mut map = D3D11_MAPPED_SUBRESOURCE::default();
    if let Err(e) = unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map)) } {
        let _ = unsafe { dup.ReleaseFrame() };
        return Err(hr_error("Map staging failed", "AcquireDupFrame", &e));
    }

    let w = capture_rect.width();
    let h = capture_rect.height();
    let row_pitch = (w * 4) as usize;
    let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];

    for y in 0..h as usize {
        unsafe {
            let src = (map.pData as *const u8).add(y * map.RowPitch as usize);
            let dst = bgra.as_mut_ptr().add(y * row_pitch);
            std::ptr::copy_nonoverlapping(src, dst, row_pitch);
        }
    }

    unsafe { context.Unmap(&staging, 0) };
    let _ = unsafe { dup.ReleaseFrame() };

    Ok(ImageBuffer {
        width: w,
        height: h,
        row_pitch: w * 4,
        origin_x: capture_rect.left,
        origin_y: capture_rect.top,
        bgra,
    })
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
    let monitor_rect = Rect {
        left: mi.rcMonitor.left,
        top: mi.rcMonitor.top,
        right: mi.rcMonitor.right,
        bottom: mi.rcMonitor.bottom,
    };

    let mut full = acquire_dup_frame(&output, &adapter, ctx.common.timeout_ms, monitor_rect)?;

    if ctx.cap.force_alpha_255 {
        for i in (3..full.bgra.len()).step_by(4) {
            full.bgra[i] = 0xFF;
        }
    }

    Ok((full, ai, oi))
}
