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
    /// Real on-disk path the capture was written to (may differ in casing from
    /// the requested path on case-insensitive volumes). Empty unless a `cap`
    /// succeeded.
    out_path: String,
}

impl Default for RunResult {
    fn default() -> Self {
        Self {
            ok: false,
            exit_code: 1,
            err: ErrorInfo::default(),
            json: String::new(),
            out_path: String::new(),
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
fn apply_dpi_mode(requested: DpiMode, logger: &Logger) -> String {
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
    logger.log(
        LogLevel::Warn,
        "SetProcessDpiAwarenessContext(PMv2) failed, fallback to system",
    );
    set_system()
}

/// The `cap` output additionally reports `client_rect_screen`; `list windows`
/// omits it.
fn window_json(w: &WindowInfo, include_client_rect: bool) -> Value {
    let mut v = json!({
        "hwnd": w.hwnd as u64,
        "pid": w.pid,
        "title": &w.title,
        "class": &w.class_name,
        "rect": w.rect,
        "visible": w.visible,
        "iconic": w.iconic,
        "cloaked": w.cloaked,
    });
    if include_client_rect {
        v.as_object_mut().unwrap().insert(
            "client_rect_screen".to_string(),
            json!(w.client_rect_screen),
        );
    }
    v
}

/// The `list monitors` output additionally reports `name`; `cap` omits it.
fn monitor_json(m: &MonitorInfo, include_name: bool) -> Value {
    let mut v = json!({
        "index": m.index,
        "desktop": m.desktop,
        "primary": m.primary,
    });
    if include_name {
        v.as_object_mut()
            .unwrap()
            .insert("name".to_string(), json!(&m.name));
    }
    v
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
            "windows": ws.iter().map(|w| window_json(w, false)).collect::<Vec<_>>(),
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
            "monitors": ms.iter().map(|m| monitor_json(m, true)).collect::<Vec<_>>(),
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
    logger: &Logger,
    ctx: &mut CaptureContext,
) -> Result<(), ErrorInfo> {
    let method = &parsed.cap.method;

    let needs_window = method.contains("window");
    if needs_window && parsed.cap.target != TargetType::Window {
        return Err(ErrorInfo::new(
            format!("method '{method}' requires --target window"),
            "RunCap",
        ));
    }

    if parsed.cap.target == TargetType::Window {
        let windows = enumerate_windows();
        let (w, reason) = resolve_window_target(&parsed.cap.window_query, &windows, logger)?;
        logger.log(
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
        ctx.window = Some(w);
    }

    if parsed.cap.target == TargetType::Screen || method.contains("monitor") {
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

        if let Some(m) = &ctx.monitor {
            logger.log(
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

/// Runs the WGC capture with retries. Non-WGC methods are rejected with a
/// validation error listing the supported methods.
fn capture_with_retry(
    parsed: &ParsedArgs,
    ctx: &CaptureContext,
    logger: &Logger,
) -> Result<ImageBuffer, ErrorInfo> {
    if !parsed.cap.method.starts_with("wgc-") {
        return Err(ErrorInfo::new(
            format!(
                "unknown method '{}' (supported: wgc-window, wgc-window2, wgc-monitor, wgc-monitor2)",
                parsed.cap.method
            ),
            "RunCap",
        ));
    }

    let mut capture_result: Result<ImageBuffer, ErrorInfo> = Err(ErrorInfo::default());

    for attempt in 0..=parsed.common.retry {
        match capture_with_wgc(ctx) {
            Ok(buf) => {
                capture_result = Ok(buf);
                break;
            }
            Err(e) => {
                logger.log(
                    LogLevel::Warn,
                    &format!(
                        "capture attempt failed attempt={} where={}",
                        attempt, e.where_
                    ),
                );
                capture_result = Err(e);
            }
        }
    }

    capture_result
}

/// Builds the success-path JSON payload for a completed capture.
#[allow(clippy::too_many_arguments)]
fn build_cap_success_json(
    parsed: &ParsedArgs,
    ctx: &CaptureContext,
    written_path: &str,
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
    js.insert("out_path".to_string(), json!(written_path));
    js.insert("format".to_string(), json!("png"));
    js.insert("timestamp".to_string(), json!(iso8601_now_local()));
    js.insert("duration_ms".to_string(), json!(duration_ms));
    js.insert("dpi_mode".to_string(), json!(dpi_applied));

    if let Some(w) = &ctx.window {
        js.insert("window".to_string(), window_json(w, true));
    }

    if let Some(m) = &ctx.monitor {
        js.insert("monitor".to_string(), monitor_json(m, false));
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

fn run_cap(parsed: &ParsedArgs, logger: &Logger, dpi_applied: &str) -> RunResult {
    let mut rr = RunResult::default();
    let start = Instant::now();

    let capture = || -> Result<(String, String), ErrorInfo> {
        let mut ctx = CaptureContext {
            cap: &parsed.cap,
            common: &parsed.common,
            window: None,
            monitor: None,
            capture_rect_screen: Rect::default(),
            logger,
        };

        resolve_capture_targets(parsed, logger, &mut ctx)?;

        let mut img = capture_with_retry(parsed, &ctx, logger)?;

        if parsed.cap.force_alpha_255 {
            for px in img.bgra.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }

        let img_rect = Rect {
            left: img.origin_x,
            top: img.origin_y,
            right: img.origin_x + img.width,
            bottom: img.origin_y + img.height,
        };
        let crop_mode = parsed.cap.crop_mode;

        let crop_rect = resolve_crop_rect_screen(
            crop_mode,
            parsed.cap.crop_rect,
            ctx.window.as_ref(),
            img_rect,
            parsed.cap.pad,
        )?;

        crop_image_in_place(crop_rect, &mut img)?;

        let stats = compute_image_stats(&img);
        logger.log(
            LogLevel::Info,
            &format!(
                "image_stats black_ratio={} transparent_ratio={}",
                stats.black_ratio, stats.transparent_ratio
            ),
        );

        save_png_wic(&img, &parsed.cap.out_path, parsed.common.overwrite)?;

        // Windows volumes are case-insensitive, so writing `test.png` when
        // `TEST.png` already exists truncates and reuses `TEST.png` -- no
        // `test.png` is created. Resolve the real on-disk path so the reported
        // success points at the file that actually exists.
        let written_path = screencap_core::encode_png::real_output_path(&parsed.cap.out_path);

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
            &written_path,
            dpi_applied,
            duration_ms,
            crop_mode,
            crop_out,
            stats,
        );

        logger.log(
            LogLevel::Info,
            &format!(
                "result=success out_path={} duration_ms={}",
                written_path, duration_ms
            ),
        );

        Ok((json, written_path))
    };

    match capture() {
        Ok((json, written_path)) => {
            rr.ok = true;
            rr.exit_code = 0;
            rr.json = json;
            rr.out_path = written_path;
        }
        Err(e) => {
            rr.err = e;
            rr.exit_code = 1;
        }
    }

    rr
}

fn log_startup(logger: &Logger, parsed: Option<&ParsedArgs>, dpi_mode: &str) {
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

fn wait_for_hotkey(parsed: &ParsedArgs, logger: &Logger) -> Result<(), ErrorInfo> {
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

    logger.log(
        LogLevel::Info,
        &format!("hotkey waiting spec={}", parsed.cap.hotkey_spec),
    );
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
            let dpi_applied = apply_dpi_mode(DpiMode::PerMonitorV2, &logger);
            log_startup(&logger, None, &dpi_applied);
            print!("{err}");
            return 0;
        }
        Err(err) => {
            let dpi_applied = apply_dpi_mode(DpiMode::PerMonitorV2, &logger);
            log_startup(&logger, None, &dpi_applied);
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

    let dpi_applied = apply_dpi_mode(parsed.common.dpi_mode, &logger);

    log_startup(&logger, Some(&parsed), &dpi_applied);

    let rr = if parsed.command == CommandType::ListWindows {
        run_list_windows(&parsed)
    } else if parsed.command == CommandType::ListMonitors {
        run_list_monitors(&parsed)
    } else if parsed.cap.hotkey_enabled {
        match wait_for_hotkey(&parsed, &logger) {
            Ok(()) => run_cap(&parsed, &logger, &dpi_applied),
            Err(e) => RunResult {
                err: e,
                exit_code: 1,
                ..Default::default()
            },
        }
    } else {
        run_cap(&parsed, &logger, &dpi_applied)
    };

    if rr.ok {
        logger.log(LogLevel::Info, "result=success");
        if parsed.common.json {
            println!("{}", rr.json);
        } else if parsed.command == CommandType::Cap {
            println!("ok: {}", rr.out_path);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_window() -> WindowInfo {
        WindowInfo {
            hwnd: 0x1234,
            pid: 42,
            title: "Title".to_string(),
            class_name: "Class".to_string(),
            rect: Rect {
                left: 1,
                top: 2,
                right: 3,
                bottom: 4,
            },
            client_rect_screen: Rect {
                left: 5,
                top: 6,
                right: 7,
                bottom: 8,
            },
            dwm_frame_rect: Rect::default(),
            visible: true,
            iconic: false,
            cloaked: false,
        }
    }

    fn sample_monitor() -> MonitorInfo {
        MonitorInfo {
            hmon: 0xABCD,
            index: 1,
            name: "\\\\.\\DISPLAY1".to_string(),
            desktop: Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            primary: true,
        }
    }

    #[test]
    fn window_json_cap_variant_includes_client_rect() {
        let v = window_json(&sample_window(), true);
        assert_eq!(v["hwnd"], json!(0x1234u64));
        assert_eq!(v["pid"], json!(42));
        assert_eq!(v["title"], json!("Title"));
        assert_eq!(v["class"], json!("Class"));
        assert!(v.get("client_rect_screen").is_some());
        assert_eq!(v["client_rect_screen"]["left"], json!(5));
    }

    #[test]
    fn window_json_list_variant_omits_client_rect() {
        let v = window_json(&sample_window(), false);
        assert!(v.get("client_rect_screen").is_none());
        // hwnd is serialized as a u64, never negative.
        assert_eq!(v["hwnd"], json!(0x1234u64));
    }

    #[test]
    fn monitor_json_list_variant_includes_name() {
        let v = monitor_json(&sample_monitor(), true);
        assert_eq!(v["index"], json!(1));
        assert_eq!(v["primary"], json!(true));
        assert_eq!(v["name"], json!("\\\\.\\DISPLAY1"));
        assert_eq!(v["desktop"]["right"], json!(1920));
    }

    #[test]
    fn monitor_json_cap_variant_omits_name() {
        let v = monitor_json(&sample_monitor(), false);
        assert!(v.get("name").is_none());
        assert_eq!(v["index"], json!(1));
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pre_parse_bootstrap_defaults_when_absent() {
        let b = pre_parse_bootstrap(&argv(&["screencap", "cap"]));
        assert_eq!(b.log_dir, "./logs");
        assert_eq!(b.log_level, LogLevel::Info);
        assert_eq!(b.command, "cap");
        assert!(!b.json);
    }

    #[test]
    fn pre_parse_bootstrap_space_separated_flags() {
        let b = pre_parse_bootstrap(&argv(&[
            "screencap",
            "cap",
            "--log-dir",
            "C:\\logs",
            "--log-level",
            "debug",
            "--json",
        ]));
        assert_eq!(b.log_dir, "C:\\logs");
        assert_eq!(b.log_level, LogLevel::Debug);
        assert!(b.json);
    }

    #[test]
    fn pre_parse_bootstrap_equals_form_flags() {
        let b = pre_parse_bootstrap(&argv(&[
            "screencap",
            "cap",
            "--log-dir=C:\\logs",
            "--log-level=warn",
        ]));
        assert_eq!(b.log_dir, "C:\\logs");
        assert_eq!(b.log_level, LogLevel::Warn);
    }

    #[test]
    fn pre_parse_bootstrap_list_windows_command_name() {
        let b = pre_parse_bootstrap(&argv(&["screencap", "list", "windows"]));
        assert_eq!(b.command, "list_windows");
    }

    #[test]
    fn build_failure_json_shape() {
        let err = ErrorInfo::new("boom", "SomeWhere");
        let s = build_failure_json("cap", "wgc-window", "window", "out.png", "system", 12, &err);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["command"], json!("cap"));
        assert_eq!(v["method"], json!("wgc-window"));
        assert_eq!(v["target"], json!("window"));
        assert_eq!(v["out_path"], json!("out.png"));
        assert_eq!(v["format"], json!("png"));
        assert_eq!(v["duration_ms"], json!(12));
        assert_eq!(v["dpi_mode"], json!("system"));
        assert_eq!(v["window"], Value::Null);
        assert_eq!(v["monitor"], Value::Null);
        assert_eq!(v["crop"], Value::Null);
        assert_eq!(v["image_stats"], Value::Null);
        // timestamp comes from a now-time call; just assert its presence.
        assert!(v.get("timestamp").is_some());
        // error is a serialized ErrorInfo object.
        assert!(v.get("error").is_some());
    }
}
