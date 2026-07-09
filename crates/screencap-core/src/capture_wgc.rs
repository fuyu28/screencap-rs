//! Windows.Graphics.Capture implementation.
//! Methods: wgc-window / wgc-window2 (window target), wgc-monitor /
//! wgc-monitor2 (monitor target). Crops to frame ContentSize, retries up to 5
//! frames until one passes the usable-frame heuristic
//! (transparent_ratio < 0.98 && black_ratio < 0.98).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::core::{IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX, D3D11_TEXTURE2D_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

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

fn wgc_log(logger: Option<&Logger>, msg: &str) {
    if let Some(l) = logger {
        l.log(LogLevel::Debug, &format!("wgc: {msg}"));
    }
}

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

fn create_capture_item_from_hwnd(hwnd: HWND) -> Result<GraphicsCaptureItem, ErrorInfo> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| to_err(e, "CreateCaptureItemFromHwnd"))?;
    unsafe { interop.CreateForWindow::<GraphicsCaptureItem>(hwnd) }
        .map_err(|e| to_err_with("CreateForWindow failed", "CreateCaptureItemFromHwnd", &e))
}

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

fn is_probably_usable_frame(img: &ImageBuffer) -> bool {
    let stats = crate::image_stats::compute_image_stats(img);
    stats.transparent_ratio < 0.98 && stats.black_ratio < 0.98
}

struct WgcResources<'a> {
    frame_pool: &'a Direct3D11CaptureFramePool,
    session: &'a GraphicsCaptureSession,
    winrt_device: &'a IDirect3DDevice,
    d3d_device: &'a ID3D11Device,
    d3d_context: &'a ID3D11DeviceContext,
}

/// Registers `FrameArrived`, starts the session, and receives up to
/// `MAX_FRAMES` frames looking for a usable one. The event handler only
/// forwards frames through a channel, so logging and image processing stay on
/// the calling thread.
fn run_capture_loop(
    ctx: &CaptureContext,
    logger: Option<&Logger>,
    res: &WgcResources,
    initial_pool_size: SizeInt32,
) -> Result<ImageBuffer, ErrorInfo> {
    let (tx, rx) = mpsc::channel::<Direct3D11CaptureFrame>();
    let handler =
        TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _args| {
            if let Some(pool) = sender.as_ref() {
                if let Ok(frame) = pool.TryGetNextFrame() {
                    let _ = tx.send(frame);
                }
            }
            Ok(())
        });
    let token = res
        .frame_pool
        .FrameArrived(&handler)
        .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    wgc_log(logger, "start capture");
    res.session
        .StartCapture()
        .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    let mut best: Option<ImageBuffer> = None;
    let mut copy_err: Option<ErrorInfo> = None;
    let mut pool_size = initial_pool_size;
    let timeout = Duration::from_millis(ctx.common.timeout_ms.max(0) as u64);
    let deadline = Instant::now() + timeout;

    for _ in 0..MAX_FRAMES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            wgc_log(logger, "timeout deadline reached");
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(frame) => {
                wgc_log(logger, "frame arrived");

                // The window can grow between pool creation (at item.Size())
                // and frame arrival; when that happens ContentSize outgrows
                // the pool's texture size, and the frame must be dropped
                // while the pool is recreated at the new size so the next
                // frame arrives un-clamped.
                if let Ok(content_size) = frame.ContentSize() {
                    if content_size.Width != pool_size.Width
                        || content_size.Height != pool_size.Height
                    {
                        wgc_log(
                            logger,
                            &format!(
                                "content size changed {}x{} -> {}x{}, recreating pool",
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
                }

                let origin = if ctx.cap.method == "wgc-window" || ctx.cap.method == "wgc-window2" {
                    ctx.window
                        .as_ref()
                        .map_or(ctx.capture_rect_screen, |w| w.dwm_frame_rect)
                } else {
                    ctx.capture_rect_screen
                };

                match copy_frame_to_image(&frame, res.d3d_device, res.d3d_context, origin) {
                    Ok(candidate) => {
                        wgc_log(
                            logger,
                            &format!("candidate size={}x{}", candidate.width, candidate.height),
                        );
                        let usable = is_probably_usable_frame(&candidate);
                        best = Some(candidate);
                        if usable {
                            break;
                        }
                    }
                    Err(e) => {
                        wgc_log(logger, &format!("copy frame failed: {}", e.message));
                        copy_err = Some(e);
                    }
                }
            }
            Err(_) => {
                wgc_log(logger, "wait did not produce frame");
            }
        }
    }

    wgc_log(logger, "revoke frame handler");
    let _ = res.frame_pool.RemoveFrameArrived(token);

    match best {
        Some(img) => Ok(img),
        None => {
            Err(copy_err.unwrap_or_else(|| ErrorInfo::new("WGC frame timeout", "CaptureWithWgc")))
        }
    }
}

pub fn capture_with_wgc(ctx: &CaptureContext) -> Result<ImageBuffer, ErrorInfo> {
    let logger = ctx.logger;

    wgc_log(logger, "init_apartment");
    // RoInitialize's HRESULT wrapper treats S_FALSE (already initialized on
    // this thread, e.g. by a prior capture attempt) as success; only a real
    // failure (such as a conflicting apartment type) is propagated.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|e| to_err(e, "CaptureWithWgc"))?;

    wgc_log(logger, "check supported");
    let supported =
        GraphicsCaptureSession::IsSupported().map_err(|e| to_err(e, "CaptureWithWgc"))?;
    if !supported {
        return Err(ErrorInfo::new(
            "GraphicsCaptureSession::IsSupported false",
            "CaptureWithWgc",
        ));
    }

    wgc_log(logger, "create d3d device");
    let (d3d_device, d3d_context) =
        create_d3d11_device(None, D3D_DRIVER_TYPE_HARDWARE, "CaptureWithWgc")?;

    wgc_log(logger, "create winrt d3d device");
    let winrt_device = create_winrt_d3d_device(&d3d_device)?;

    let item =
        if ctx.cap.method == "wgc-window" || ctx.cap.method == "wgc-window2" {
            wgc_log(logger, "create item for window");
            let window = ctx.window.as_ref().ok_or_else(|| {
                ErrorInfo::new("wgc-window needs window target", "CaptureWithWgc")
            })?;
            create_capture_item_from_hwnd(HWND(window.hwnd as *mut core::ffi::c_void))?
        } else if ctx.cap.method == "wgc-monitor" || ctx.cap.method == "wgc-monitor2" {
            wgc_log(logger, "create item for monitor");
            let monitor = ctx.monitor.as_ref().ok_or_else(|| {
                ErrorInfo::new("wgc-monitor needs monitor target", "CaptureWithWgc")
            })?;
            create_capture_item_from_monitor(HMONITOR(monitor.hmon as *mut core::ffi::c_void))?
        } else {
            return Err(ErrorInfo::new("unknown wgc method", "CaptureWithWgc"));
        };

    let size = item.Size().map_err(|e| to_err(e, "CaptureWithWgc"))?;
    wgc_log(logger, &format!("item size={}x{}", size.Width, size.Height));

    wgc_log(logger, "create frame pool");
    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt_device,
        FRAME_POOL_PIXEL_FORMAT,
        FRAME_POOL_BUFFERS,
        size,
    )
    .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    wgc_log(logger, "create session");
    let session = frame_pool
        .CreateCaptureSession(&item)
        .map_err(|e| to_err(e, "CaptureWithWgc"))?;

    // From here on, session/frame_pool exist, so we always close them before
    // returning -- whatever run_capture_loop produces (Ok or Err) is
    // returned only after cleanup runs.
    let result = run_capture_loop(
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
    );

    wgc_log(logger, "close session");
    let _ = session.Close();
    wgc_log(logger, "close frame pool");
    let _ = frame_pool.Close();

    result
}
