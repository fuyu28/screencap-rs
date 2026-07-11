//! Windows.Graphics.Capture implementation.
//! Methods: wgc-window (window target), wgc-monitor (monitor target). Crops to
//! frame ContentSize, retries up to 5 frames until one passes the usable-frame
//! heuristic (transparent_ratio < 0.98 && black_ratio < 0.98).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_TEXTURE2D_DESC, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{IInspectable, Interface};

use crate::d3d11_copy::{copy_texture_to_image, create_d3d11_device};
use crate::logging::Logger;
use crate::types::{CaptureContext, ErrorInfo, ImageBuffer, LogLevel, Rect};

const MAX_FRAMES: usize = 5;
const FRAME_POOL_BUFFERS: i32 = 1;
const FRAME_POOL_PIXEL_FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

fn to_err(e: windows::core::Error, where_: &str) -> ErrorInfo {
    ErrorInfo::with_hresult(e.message(), where_, e.code().0 as u32)
}

fn to_err_with(message: &str, where_: &str, e: &windows::core::Error) -> ErrorInfo {
    ErrorInfo::with_hresult(message, where_, e.code().0 as u32)
}

/// Wraps a D3D11 device as the WinRT `IDirect3DDevice` WGC expects.
fn create_winrt_d3d_device(d3d_device: &ID3D11Device) -> Result<IDirect3DDevice, ErrorInfo> {
    let dxgi_device: IDXGIDevice = d3d_device.cast().map_err(|e| {
        to_err_with(
            "QueryInterface IDXGIDevice failed",
            "CreateWinRtD3DDevice",
            &e,
        )
    })?;
    let inspectable: IInspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .map_err(|e| {
        to_err_with(
            "CreateDirect3D11DeviceFromDXGIDevice failed",
            "CreateWinRtD3DDevice",
            &e,
        )
    })?;
    inspectable
        .cast::<IDirect3DDevice>()
        .map_err(|e| to_err(e, "CreateWinRtD3DDevice"))
}

/// Builds a WGC capture item for the given top-level window.
fn create_capture_item_from_hwnd(hwnd: HWND) -> Result<GraphicsCaptureItem, ErrorInfo> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| to_err(e, "CreateCaptureItemFromHwnd"))?;
    unsafe { interop.CreateForWindow::<GraphicsCaptureItem>(hwnd) }
        .map_err(|e| to_err_with("CreateForWindow failed", "CreateCaptureItemFromHwnd", &e))
}

/// Builds a WGC capture item for the given display monitor.
fn create_capture_item_from_monitor(hmon: HMONITOR) -> Result<GraphicsCaptureItem, ErrorInfo> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| to_err(e, "CreateCaptureItemFromMonitor"))?;
    unsafe { interop.CreateForMonitor::<GraphicsCaptureItem>(hmon) }.map_err(|e| {
        to_err_with(
            "CreateForMonitor failed",
            "CreateCaptureItemFromMonitor",
            &e,
        )
    })
}

/// Copies a captured frame into a plain BGRA buffer, cropped to the frame's
/// reported `ContentSize` (WGC pads the underlying texture to the swapchain
/// size, so the content can be smaller than the texture).
fn copy_frame_to_image(
    frame: &Direct3D11CaptureFrame,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    origin_rect: Rect,
) -> Result<ImageBuffer, ErrorInfo> {
    let surface = frame.Surface().map_err(|e| to_err(e, "CopyFrameToImage"))?;
    let access: IDirect3DDxgiInterfaceAccess =
        surface.cast().map_err(|e| to_err(e, "CopyFrameToImage"))?;
    let tex: ID3D11Texture2D =
        unsafe { access.GetInterface::<ID3D11Texture2D>() }.map_err(|e| {
            to_err_with(
                "GetInterface(ID3D11Texture2D) failed",
                "CopyFrameToImage",
                &e,
            )
        })?;

    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { tex.GetDesc(&mut desc) };

    let content_size = frame
        .ContentSize()
        .map_err(|e| to_err(e, "CopyFrameToImage"))?;
    if content_size.Width <= 0 || content_size.Height <= 0 {
        return Err(ErrorInfo::new(
            "invalid WGC ContentSize",
            "CopyFrameToImage",
        ));
    }
    let width = desc.Width.min(content_size.Width as u32);
    let height = desc.Height.min(content_size.Height as u32);

    let src_box = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: width,
        bottom: height,
        back: 1,
    };

    let rect = Rect {
        left: origin_rect.left,
        top: origin_rect.top,
        right: origin_rect.left + width as i32,
        bottom: origin_rect.top + height as i32,
    };

    copy_texture_to_image(
        device,
        context,
        desc,
        rect,
        |staging| unsafe {
            context.CopySubresourceRegion(staging, 0, 0, 0, 0, &tex, 0, Some(&src_box));
        },
        "CopyFrameToImage",
    )
}

/// Returns true when `method` targets a window (as opposed to a monitor).
fn is_wgc_window_method(method: &str) -> bool {
    method == "wgc-window"
}

/// Heuristic filter for WGC warm-up frames (mostly black or transparent).
fn is_probably_usable_frame(img: &ImageBuffer) -> bool {
    let (black_ratio, transparent_ratio) = crate::image_stats::compute_frame_ratios(img);
    transparent_ratio < 0.98 && black_ratio < 0.98
}

/// Open WGC session handles shared by [`run_capture_loop`].
struct WgcResources<'a> {
    frame_pool: &'a Direct3D11CaptureFramePool,
    session: &'a GraphicsCaptureSession,
    winrt_device: &'a IDirect3DDevice,
    d3d_device: &'a ID3D11Device,
    d3d_context: &'a ID3D11DeviceContext,
}

/// Registers `FrameArrived`, starts the session, and receives up to
/// `MAX_FRAMES` frames looking for a usable one. Uses the same screen origin
/// for every iteration in the loop.
fn run_capture_loop(
    ctx: &CaptureContext,
    logger: &Logger,
    res: &WgcResources,
    initial_pool_size: SizeInt32,
) -> Result<ImageBuffer, ErrorInfo> {
    let (tx, rx) = mpsc::channel::<Direct3D11CaptureFrame>();
    let handler =
        TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _args| {
            if let Some(pool) = sender.as_ref()
                && let Ok(frame) = pool.TryGetNextFrame()
            {
                let _ = tx.send(frame);
            }
            Ok(())
        });
    let token = res
        .frame_pool
        .FrameArrived(&handler)
        .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    // Do not leave FrameArrived registered after StartCapture fails; Close still
    // runs in the caller but an attached handler is unnecessary work on that path.
    /// Unregisters the WGC `FrameArrived` handler when capture setup or the loop ends.
    struct FrameArrivedGuard<'a> {
        pool: &'a Direct3D11CaptureFramePool,
        token: i64,
    }
    impl Drop for FrameArrivedGuard<'_> {
        fn drop(&mut self) {
            let _ = self.pool.RemoveFrameArrived(self.token);
        }
    }
    let _arrived_guard = FrameArrivedGuard {
        pool: res.frame_pool,
        token,
    };

    res.session
        .StartCapture()
        .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    let mut best: Option<ImageBuffer> = None;
    let mut copy_err: Option<ErrorInfo> = None;
    let mut pool_size = initial_pool_size;
    let timeout = Duration::from_millis(ctx.common.timeout_ms.max(0) as u64);
    let deadline = Instant::now() + timeout;

    let origin = if is_wgc_window_method(&ctx.cap.method) {
        ctx.window
            .as_ref()
            .map_or(ctx.capture_rect_screen, |w| w.dwm_frame_rect)
    } else {
        ctx.capture_rect_screen
    };

    for _ in 0..MAX_FRAMES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            logger.log(LogLevel::Debug, "wgc: timeout deadline reached");
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(frame) => {
                // Do not copy into a pool sized at item.Size() when ContentSize
                // grew: recreate the pool first or the frame stays clamped.
                if let Ok(content_size) = frame.ContentSize()
                    && (content_size.Width != pool_size.Width
                        || content_size.Height != pool_size.Height)
                {
                    logger.log(
                        LogLevel::Debug,
                        &format!(
                            "wgc: content size changed {}x{} -> {}x{}, recreating pool",
                            pool_size.Width,
                            pool_size.Height,
                            content_size.Width,
                            content_size.Height
                        ),
                    );
                    match res.frame_pool.Recreate(
                        res.winrt_device,
                        FRAME_POOL_PIXEL_FORMAT,
                        FRAME_POOL_BUFFERS,
                        content_size,
                    ) {
                        Ok(()) => pool_size = content_size,
                        Err(e) => copy_err = Some(to_err(e, "CaptureWithWgc")),
                    }
                    continue;
                }

                match copy_frame_to_image(&frame, res.d3d_device, res.d3d_context, origin) {
                    Ok(candidate) => {
                        logger.log(
                            LogLevel::Debug,
                            &format!(
                                "wgc: candidate size={}x{}",
                                candidate.width, candidate.height
                            ),
                        );
                        let usable = is_probably_usable_frame(&candidate);
                        best = Some(candidate);
                        if usable {
                            break;
                        }
                    }
                    Err(e) => {
                        logger.log(
                            LogLevel::Debug,
                            &format!("wgc: copy frame failed: {}", e.message),
                        );
                        copy_err = Some(e);
                    }
                }
            }
            Err(_) => {
                logger.log(LogLevel::Debug, "wgc: wait did not produce frame");
            }
        }
    }

    match best {
        Some(img) => Ok(img),
        None => {
            Err(copy_err.unwrap_or_else(|| ErrorInfo::new("WGC frame timeout", "CaptureWithWgc")))
        }
    }
}

/// Captures one frame via WGC for the target in `ctx`, returning a BGRA buffer.
pub fn capture_with_wgc(ctx: &CaptureContext) -> Result<ImageBuffer, ErrorInfo> {
    let logger = ctx.logger;

    // Do not treat RoInitialize S_FALSE (already initialized on this thread) as
    // failure; only a conflicting apartment type is propagated.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|e| to_err(e, "CaptureWithWgc"))?;

    let supported =
        GraphicsCaptureSession::IsSupported().map_err(|e| to_err(e, "CaptureWithWgc"))?;
    if !supported {
        return Err(ErrorInfo::new(
            "GraphicsCaptureSession::IsSupported false",
            "CaptureWithWgc",
        ));
    }

    let (d3d_device, d3d_context) = create_d3d11_device("CaptureWithWgc")?;

    let winrt_device = create_winrt_d3d_device(&d3d_device)?;

    let item =
        if is_wgc_window_method(&ctx.cap.method) {
            let window = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new("wgc-window needs window target", "CaptureWithWgc")
            })?;
            create_capture_item_from_hwnd(HWND(window.hwnd as *mut core::ffi::c_void))?
        } else if ctx.cap.method == "wgc-monitor" {
            let monitor = ctx.monitor.as_ref().ok_or_else(|| {
                ErrorInfo::new("wgc-monitor needs monitor target", "CaptureWithWgc")
            })?;
            create_capture_item_from_monitor(HMONITOR(monitor.hmon as *mut core::ffi::c_void))?
        } else {
            return Err(ErrorInfo::new("unknown wgc method", "CaptureWithWgc"));
        };

    let size = item.Size().map_err(|e| to_err(e, "CaptureWithWgc"))?;

    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt_device,
        FRAME_POOL_PIXEL_FORMAT,
        FRAME_POOL_BUFFERS,
        size,
    )
    .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    let session = frame_pool
        .CreateCaptureSession(&item)
        .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    // Do not call SetIsCursorCaptureEnabled when the cursor is requested: WGC
    // includes it by default and the property is missing on pre-1903 builds.
    // Exclusion uses the property and fails clearly where it is unavailable.
    //
    // Do not early-return before Close: funnel property failure through the
    // and_then chain below so session/frame_pool always close on every path.
    let result = if ctx.cap.include_cursor {
        Ok(())
    } else {
        session.SetIsCursorCaptureEnabled(false).map_err(|e| {
            to_err_with(
                "SetIsCursorCaptureEnabled failed (cursor exclusion requires Windows 10 version 1903 / build 18362 or later; pass --cursor to include the cursor instead)",
                "CaptureWithWgc",
                &e,
            )
        })
    }
    .and_then(|()| {
        run_capture_loop(
            ctx,
            logger,
            &WgcResources {
                frame_pool: &frame_pool,
                session: &session,
                winrt_device: &winrt_device,
                d3d_device: &d3d_device,
                d3d_context: &d3d_context,
            },
            size,
        )
    });

    let _ = session.Close();
    let _ = frame_pool.Close();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: i32, height: i32, pixel: [u8; 4]) -> ImageBuffer {
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            bgra.extend_from_slice(&pixel);
        }
        ImageBuffer {
            width,
            height,
            row_pitch: width * 4,
            origin_x: 0,
            origin_y: 0,
            bgra,
        }
    }

    #[test]
    fn wgc_window_methods_recognised() {
        assert!(is_wgc_window_method("wgc-window"));
    }

    #[test]
    fn non_window_methods_rejected() {
        assert!(!is_wgc_window_method("wgc-monitor"));
        assert!(!is_wgc_window_method("wgc-window2"));
        assert!(!is_wgc_window_method("wgc-monitor2"));
        assert!(!is_wgc_window_method("dxgi-window"));
        assert!(!is_wgc_window_method(""));
    }

    #[test]
    fn usable_frame_accepts_normal_content() {
        let img = solid(4, 4, [10, 20, 30, 255]);
        assert!(is_probably_usable_frame(&img));
    }

    #[test]
    fn usable_frame_rejects_fully_black() {
        let img = solid(4, 4, [0, 0, 0, 255]);
        assert!(!is_probably_usable_frame(&img));
    }

    #[test]
    fn usable_frame_rejects_fully_transparent() {
        let img = solid(4, 4, [10, 20, 30, 0]);
        assert!(!is_probably_usable_frame(&img));
    }
}
