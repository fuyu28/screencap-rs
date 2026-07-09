//! CLI runtime: logging, DPI mode, command dispatch, hotkey wait, JSON output,
//! and process exit codes.

use std::time::Instant;

use serde_json::{json, Map, Value};
use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, GetSystemMetrics, SetProcessDPIAware, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WM_HOTKEY,
};

use screencap_core::capture_dxgi::capture_with_dxgi;
use screencap_core::capture_gdi::capture_with_gdi;
use screencap_core::capture_wgc::capture_with_wgc;
use screencap_core::crop::{crop_image_in_place, resolve_crop_rect_screen};
use screencap_core::encode_png::save_png_wic;
use screencap_core::image_stats::compute_image_stats;
use screencap_core::logging::{get_build_stamp, get_os_version_string, Logger};
use screencap_core::monitor_enum::{enumerate_monitors, find_monitor_by_token};
use screencap_core::types::*;
use screencap_core::util::iso8601_now_local;
use screencap_core::window_enum::{enumerate_windows, resolve_window_target};

use crate::cli::{self, ParsedArgs};

const HOTKEY_ID: i32 = 0x5343;

struct RunResult {
    ok: bool,
    exit_code: i32,
    err: ErrorInfo,
    json: String,
}

impl Default for RunResult {
    fn default() -> Self {
        Self {
            ok: false,
            exit_code: 1,
            err: ErrorInfo::default(),
            json: String::new(),
        }
    }
}

struct BootstrapOptions {
    log_dir: String,
    log_level: LogLevel,
    command: String,
    json: bool,
}

fn pre_parse_bootstrap(argv: &[String]) -> BootstrapOptions {
    let mut b = BootstrapOptions {
        log_dir: "./logs".to_string(),
        log_level: LogLevel::Info,
        command: "unknown".to_string(),
        json: false,
    };

    let argc = argv.len();
    if argc >= 2 {
        b.command = argv[1].clone();
        if b.command == "list" && argc >= 3 {
            b.command = format!("list_{}", argv[2]);
        }
    }

    let mut i = 1usize;
    while i < argc {
        let a = &argv[i];
        if a == "--log-dir" && i + 1 < argc {
            i += 1;
            b.log_dir = argv[i].clone();
        } else if let Some(value) = a.strip_prefix("--log-dir=") {
            b.log_dir = value.to_string();
        } else if a == "--log-level" && i + 1 < argc {
            i += 1;
            b.log_level = screencap_core::logging::parse_log_level(&argv[i]);
        } else if let Some(value) = a.strip_prefix("--log-level=") {
            b.log_level = screencap_core::logging::parse_log_level(value);
        } else if a == "--json" {
            b.json = true;
        }
        i += 1;
    }

    b
}

/// Returns the applied DPI mode string ("per-monitor-v2" or "system").
fn apply_dpi_mode(requested: DpiMode, logger: Option<&Logger>) -> String {
    fn set_system() -> String {
        unsafe {
            let _ = SetProcessDPIAware();
        }
        "system".to_string()
    }

    if requested == DpiMode::System {
        return set_system();
    }

    let result =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if result.is_ok() {
        return "per-monitor-v2".to_string();
    }
    if let Some(lg) = logger {
        lg.log(
            LogLevel::Warn,
            "SetProcessDpiAwarenessContext(PMv2) failed, fallback to system",
        );
    }
    set_system()
}

fn window_json(w: &WindowInfo) -> Value {
    json!({
        "hwnd": w.hwnd as u64,
        "pid": w.pid,
        "title": &w.title,
        "class": &w.class_name,
        "rect": w.rect,
        "visible": w.visible,
        "iconic": w.iconic,
        "cloaked": w.cloaked,
    })
}

fn cap_window_json(w: &WindowInfo) -> Value {
    json!({
        "hwnd": w.hwnd as u64,
        "pid": w.pid,
        "title": &w.title,
        "class": &w.class_name,
        "rect": w.rect,
        "visible": w.visible,
        "iconic": w.iconic,
        "cloaked": w.cloaked,
        "client_rect_screen": w.client_rect_screen,
    })
}

fn list_monitor_json(m: &MonitorInfo) -> Value {
    json!({
        "index": m.index,
        "name": &m.name,
        "desktop": m.desktop,
        "primary": m.primary,
    })
}

fn cap_monitor_json(m: &MonitorInfo) -> Value {
    json!({
        "index": m.index,
        "desktop": m.desktop,
        "primary": m.primary,
    })
}

fn run_list_windows(parsed: &ParsedArgs) -> RunResult {
    let mut rr = RunResult::default();
    let ws = enumerate_windows();
    rr.ok = true;
    rr.exit_code = 0;

    if parsed.common.json {
        rr.json = json!({
            "ok": true,
            "command": "list windows",
            "timestamp": iso8601_now_local(),
            "windows": ws.iter().map(window_json).collect::<Vec<_>>(),
        })
        .to_string();
    } else {
        println!("windows={}", ws.len());
        for w in &ws {
            println!(
                "hwnd={} pid={} title={} class={} rect={},{},{},{} visible={} iconic={} cloaked={}",
                w.hwnd as u64,
                w.pid,
                w.title,
                w.class_name,
                w.rect.left,
                w.rect.top,
                w.rect.right,
                w.rect.bottom,
                w.visible as i32,
                w.iconic as i32,
                w.cloaked as i32,
            );
        }
    }
    rr
}

fn run_list_monitors(parsed: &ParsedArgs) -> RunResult {
    let mut rr = RunResult::default();
    let ms = enumerate_monitors();
    rr.ok = true;
    rr.exit_code = 0;

    if parsed.common.json {
        rr.json = json!({
            "ok": true,
            "command": "list monitors",
            "timestamp": iso8601_now_local(),
            "monitors": ms.iter().map(list_monitor_json).collect::<Vec<_>>(),
        })
        .to_string();
    } else {
        println!("monitors={}", ms.len());
        for m in &ms {
            println!(
                "index={} name={} rect={},{},{},{} primary={}",
                m.index,
                m.name,
                m.desktop.left,
                m.desktop.top,
                m.desktop.right,
                m.desktop.bottom,
                m.primary as i32,
            );
        }
    }
    rr
}

fn virtual_screen_rect() -> Rect {
    unsafe {
        let l = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let t = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        Rect {
            left: l,
            top: t,
            right: l + w,
            bottom: t + h,
        }
    }
}

/// Resolves the window/monitor targets for a capture and fills in
/// `ctx.window`, `ctx.monitor`, and `ctx.capture_rect_screen`.
fn resolve_capture_targets(
    parsed: &ParsedArgs,
    logger: Option<&Logger>,
    ctx: &mut CaptureContext,
) -> Result<(), ErrorInfo> {
    let method = &parsed.cap.method;

    let needs_window = method.contains("window") || method.contains("client");
    if needs_window && parsed.cap.target != TargetType::Window {
        return Err(ErrorInfo::new(
            format!("method '{method}' requires --target window"),
            "RunCap",
        ));
    }

    if parsed.cap.target == TargetType::Window {
        let windows = enumerate_windows();
        let (w, reason) = resolve_window_target(&parsed.cap.window_query, &windows, logger)?;
        if let Some(lg) = logger {
            lg.log(
                LogLevel::Info,
                &format!(
                    "resolved window hwnd={} pid={} title={} class={} rect={},{},{},{} visible={} iconic={} cloaked={} reason={}",
                    w.hwnd as u64,
                    w.pid,
                    w.title,
                    w.class_name,
                    w.rect.left,
                    w.rect.top,
                    w.rect.right,
                    w.rect.bottom,
                    if w.visible { 1 } else { 0 },
                    if w.iconic { 1 } else { 0 },
                    if w.cloaked { 1 } else { 0 },
                    reason,
                ),
            );
        }
        ctx.window = Some(w);
    }

    if parsed.cap.target == TargetType::Screen
        || method.contains("monitor")
        || method == "dxgi-window"
    {
        let monitors = enumerate_monitors();
        if parsed.cap.screen_query.virtual_screen {
            ctx.capture_rect_screen = virtual_screen_rect();
        } else if let Some(token) = &parsed.cap.screen_query.monitor {
            match find_monitor_by_token(&monitors, token) {
                Some(mon) => {
                    ctx.capture_rect_screen = mon.desktop;
                    ctx.monitor = Some(mon);
                }
                None => {
                    return Err(ErrorInfo::new("monitor not found", "RunCap"));
                }
            }
        } else if let Some(w) = &ctx.window {
            let h = unsafe { MonitorFromWindow(HWND(w.hwnd as *mut _), MONITOR_DEFAULTTONEAREST) };
            for m in &monitors {
                if m.hmon == h.0 as isize {
                    ctx.monitor = Some(m.clone());
                    ctx.capture_rect_screen = m.desktop;
                    break;
                }
            }
        }

        if let (Some(lg), Some(m)) = (logger, &ctx.monitor) {
            lg.log(
                LogLevel::Info,
                &format!(
                    "resolved monitor index={} rect={},{},{},{} primary={}",
                    m.index,
                    m.desktop.left,
                    m.desktop.top,
                    m.desktop.right,
                    m.desktop.bottom,
                    if m.primary { 1 } else { 0 },
                ),
            );
        }
    }

    if !ctx.capture_rect_screen.is_valid() {
        if let Some(w) = &ctx.window {
            ctx.capture_rect_screen = w.rect;
        }
    }

    Ok(())
}

/// Runs the capture method with retries, returning the captured image plus
/// the DXGI adapter/output indices (only meaningful for `dxgi-*` methods).
fn capture_with_retry(
    parsed: &ParsedArgs,
    ctx: &CaptureContext,
    logger: Option<&Logger>,
) -> (Result<ImageBuffer, ErrorInfo>, i32, i32) {
    let mut adapter_index: i32 = -1;
    let mut output_index: i32 = -1;
    let mut capture_result: Result<ImageBuffer, ErrorInfo> = Err(ErrorInfo::default());

    for attempt in 0..=parsed.common.retry {
        let result: Result<ImageBuffer, ErrorInfo> = if parsed.cap.method.starts_with("gdi-") {
            capture_with_gdi(ctx)
        } else if parsed.cap.method.starts_with("dxgi-") {
            match capture_with_dxgi(ctx) {
                Ok((buf, a, o)) => {
                    adapter_index = a;
                    output_index = o;
                    Ok(buf)
                }
                Err(e) => Err(e),
            }
        } else if parsed.cap.method.starts_with("wgc-") {
            capture_with_wgc(ctx)
        } else {
            Err(ErrorInfo::new("unknown method", "RunCap"))
        };

        match result {
            Ok(buf) => {
                capture_result = Ok(buf);
                break;
            }
            Err(e) => {
                if let Some(lg) = logger {
                    lg.log(
                        LogLevel::Warn,
                        &format!(
                            "capture attempt failed attempt={} where={}",
                            attempt, e.where_
                        ),
                    );
                }
                capture_result = Err(e);
            }
        }
    }

    (capture_result, adapter_index, output_index)
}

/// Builds the success-path JSON payload for a completed capture.
#[allow(clippy::too_many_arguments)]
fn build_cap_success_json(
    parsed: &ParsedArgs,
    ctx: &CaptureContext,
    dpi_applied: &str,
    duration_ms: i32,
    crop_mode: CropMode,
    crop_out: CropRect,
    stats: ImageStats,
) -> String {
    let mut js = Map::new();
    js.insert("ok".to_string(), json!(true));
    js.insert("command".to_string(), json!("cap"));
    js.insert("method".to_string(), json!(&parsed.cap.method));
    js.insert(
        "target".to_string(),
        json!(cli::target_type_name(parsed.cap.target)),
    );
    js.insert("out_path".to_string(), json!(&parsed.cap.out_path));
    js.insert("format".to_string(), json!("png"));
    js.insert("timestamp".to_string(), json!(iso8601_now_local()));
    js.insert("duration_ms".to_string(), json!(duration_ms));
    js.insert("dpi_mode".to_string(), json!(dpi_applied));

    if let Some(w) = &ctx.window {
        js.insert("window".to_string(), cap_window_json(w));
    }

    if let Some(m) = &ctx.monitor {
        js.insert("monitor".to_string(), cap_monitor_json(m));
    }

    js.insert(
        "crop".to_string(),
        json!({
            "mode": cli::crop_mode_name(crop_mode),
            "rect": crop_out,
            "pad": parsed.cap.pad,
        }),
    );
    js.insert("image_stats".to_string(), json!(stats));
    js.insert("error".to_string(), Value::Null);

    Value::Object(js).to_string()
}

fn run_cap(parsed: &ParsedArgs, logger: Option<&Logger>, dpi_applied: &str) -> RunResult {
    let mut rr = RunResult::default();
    let start = Instant::now();

    let capture = || -> Result<String, ErrorInfo> {
        let mut ctx = CaptureContext {
            cap: &parsed.cap,
            common: &parsed.common,
            window: None,
            monitor: None,
            capture_rect_screen: Rect::default(),
            logger,
        };

        resolve_capture_targets(parsed, logger, &mut ctx)?;

        let (capture_result, adapter_index, output_index) =
            capture_with_retry(parsed, &ctx, logger);
        let mut img = capture_result?;

        if parsed.cap.force_alpha_255 {
            for px in img.bgra.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }

        if let Some(lg) = logger {
            if parsed.cap.method.starts_with("dxgi-") {
                lg.log(
                    LogLevel::Info,
                    &format!(
                        "DXGI adapter_index={} output_index={} frame_size={}x{} row_pitch={}",
                        adapter_index, output_index, img.width, img.height, img.row_pitch
                    ),
                );
            }
        }

        let img_rect = Rect {
            left: img.origin_x,
            top: img.origin_y,
            right: img.origin_x + img.width,
            bottom: img.origin_y + img.height,
        };
        let mut crop_mode = parsed.cap.crop_mode;
        if crop_mode == CropMode::None && parsed.cap.method == "dxgi-window" {
            crop_mode = CropMode::Window;
        }

        let crop_rect = resolve_crop_rect_screen(
            crop_mode,
            parsed.cap.crop_rect,
            ctx.window.as_ref(),
            img_rect,
            parsed.cap.pad,
        )?;

        crop_image_in_place(crop_rect, &mut img)?;

        let stats = compute_image_stats(&img);
        if let Some(lg) = logger {
            lg.log(
                LogLevel::Info,
                &format!(
                    "image_stats black_ratio={} transparent_ratio={}",
                    stats.black_ratio, stats.transparent_ratio
                ),
            );
        }

        save_png_wic(&img, &parsed.cap.out_path, parsed.common.overwrite)?;

        let duration_ms = start.elapsed().as_millis() as i32;

        let crop_out = CropRect {
            x: img.origin_x,
            y: img.origin_y,
            w: img.width,
            h: img.height,
        };

        let json = build_cap_success_json(
            parsed,
            &ctx,
            dpi_applied,
            duration_ms,
            crop_mode,
            crop_out,
            stats,
        );

        if let Some(lg) = logger {
            lg.log(
                LogLevel::Info,
                &format!(
                    "result=success out_path={} duration_ms={}",
                    parsed.cap.out_path, duration_ms
                ),
            );
        }

        Ok(json)
    };

    match capture() {
        Ok(json) => {
            rr.ok = true;
            rr.exit_code = 0;
            rr.json = json;
        }
        Err(e) => {
            rr.err = e;
            rr.exit_code = 1;
        }
    }

    rr
}

fn log_startup(logger: Option<&Logger>, parsed: Option<&ParsedArgs>, dpi_mode: &str) {
    let logger = match logger {
        Some(l) => l,
        None => return,
    };
    logger.log(LogLevel::Info, &format!("version={VERSION}"));
    logger.log(LogLevel::Info, &format!("build={}", get_build_stamp()));
    logger.log(LogLevel::Info, &format!("os={}", get_os_version_string()));
    logger.log(LogLevel::Info, &format!("dpi_mode={dpi_mode}"));
    if let Some(p) = parsed {
        logger.log(LogLevel::Info, &format!("argv={}", p.raw_args));
    }
}

fn build_failure_json(
    command: &str,
    method: &str,
    target: &str,
    out_path: &str,
    dpi_mode: &str,
    duration_ms: i32,
    err: &ErrorInfo,
) -> String {
    json!({
        "ok": false,
        "command": command,
        "method": method,
        "target": target,
        "out_path": out_path,
        "format": "png",
        "timestamp": iso8601_now_local(),
        "duration_ms": duration_ms,
        "dpi_mode": dpi_mode,
        "window": Value::Null,
        "monitor": Value::Null,
        "crop": Value::Null,
        "image_stats": Value::Null,
        "error": err,
    })
    .to_string()
}

fn wait_for_hotkey(parsed: &ParsedArgs, logger: Option<&Logger>) -> Result<(), ErrorInfo> {
    if !parsed.cap.hotkey_enabled {
        return Ok(());
    }

    if let Err(_e) = unsafe {
        RegisterHotKey(
            None,
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(parsed.cap.hotkey_modifiers),
            parsed.cap.hotkey_vk,
        )
    } {
        let code = unsafe { GetLastError() };
        return Err(ErrorInfo::with_win32(
            "RegisterHotKey failed",
            "WaitForHotkey",
            code.0,
        ));
    }

    if let Some(lg) = logger {
        lg.log(
            LogLevel::Info,
            &format!("hotkey waiting spec={}", parsed.cap.hotkey_spec),
        );
    }
    if !parsed.common.json {
        println!("waiting hotkey: {}", parsed.cap.hotkey_spec);
    }

    let mut msg = MSG::default();
    let result: Result<(), ErrorInfo>;
    loop {
        let gm = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if gm.0 == -1 {
            let code = unsafe { GetLastError() };
            result = Err(ErrorInfo::with_win32(
                "GetMessage failed",
                "WaitForHotkey",
                code.0,
            ));
            break;
        }
        if gm.0 == 0 {
            result = Err(ErrorInfo::new(
                "message loop ended before hotkey",
                "WaitForHotkey",
            ));
            break;
        }
        if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == HOTKEY_ID {
            result = Ok(());
            break;
        }
    }

    unsafe {
        let _ = UnregisterHotKey(None, HOTKEY_ID);
    }

    if result.is_ok() && !parsed.common.json {
        println!("hotkey pressed");
    }

    result
}

/// Whole-program run; returns the process exit code.
pub fn run() -> i32 {
    let argv: Vec<String> = std::env::args().collect();

    let boot = pre_parse_bootstrap(&argv);
    let mut logger = Logger::new();
    if let Err(err) = logger.init(&boot.log_dir, &boot.command, boot.log_level) {
        if !boot.json {
            eprintln!("Warning: failed to initialize logger: {err}");
        }
    }

    let parsed = match cli::parse_args(&argv) {
        Ok(parsed) => parsed,
        Err(err) if cli::is_help_error(&err) => {
            let dpi_applied = apply_dpi_mode(DpiMode::PerMonitorV2, Some(&logger));
            log_startup(Some(&logger), None, &dpi_applied);
            print!("{err}");
            return 0;
        }
        Err(err) => {
            let dpi_applied = apply_dpi_mode(DpiMode::PerMonitorV2, Some(&logger));
            log_startup(Some(&logger), None, &dpi_applied);
            let error = err.to_string();
            logger.log(LogLevel::Error, &format!("parse error: {error}"));
            if boot.json {
                let err = ErrorInfo::new(error, "ParseArgs");
                println!(
                    "{}",
                    build_failure_json("unknown", "", "", "", &dpi_applied, 0, &err)
                );
            } else {
                eprint!("{err}");
            }
            return 2;
        }
    };

    let dpi_applied = apply_dpi_mode(parsed.common.dpi_mode, Some(&logger));

    log_startup(Some(&logger), Some(&parsed), &dpi_applied);

    let rr = if parsed.command == CommandType::ListWindows {
        run_list_windows(&parsed)
    } else if parsed.command == CommandType::ListMonitors {
        run_list_monitors(&parsed)
    } else if parsed.cap.hotkey_enabled {
        match wait_for_hotkey(&parsed, Some(&logger)) {
            Ok(()) => run_cap(&parsed, Some(&logger), &dpi_applied),
            Err(e) => RunResult {
                err: e,
                exit_code: 1,
                ..Default::default()
            },
        }
    } else {
        run_cap(&parsed, Some(&logger), &dpi_applied)
    };

    if rr.ok {
        logger.log(LogLevel::Info, "result=success");
        if parsed.common.json {
            println!("{}", rr.json);
        } else if parsed.command == CommandType::Cap {
            println!("ok: {}", parsed.cap.out_path);
        }
        return rr.exit_code;
    }

    logger.log(LogLevel::Error, &format!("result=failure error={}", rr.err));

    if parsed.common.json || parsed.command == CommandType::Cap {
        let command_str = if parsed.command == CommandType::Cap {
            "cap"
        } else {
            "list"
        };
        println!(
            "{}",
            build_failure_json(
                command_str,
                &parsed.cap.method,
                cli::target_type_name(parsed.cap.target),
                &parsed.cap.out_path,
                &dpi_applied,
                0,
                &rr.err,
            )
        );
    } else {
        eprintln!("Error: {}", rr.err);
    }

    rr.exit_code
}
