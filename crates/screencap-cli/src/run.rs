//! Port of the run logic in src/main.cpp: bootstrap logging, DPI mode,
//! command dispatch (cap / list windows / list monitors), hotkey wait,
//! JSON output, exit codes (0 ok, 1 runtime failure, 2 parse error).

use std::time::Instant;

use windows::Win32::Foundation::{GetLastError, HWND};
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, GetSystemMetrics, SetProcessDPIAware, MSG, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WM_HOTKEY,
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
use screencap_core::util::{iso8601_now_local, json_escape, to_hex32};
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
        } else if a == "--log-level" && i + 1 < argc {
            i += 1;
            b.log_level = screencap_core::logging::parse_log_level(&argv[i]);
        } else if a == "--json" {
            b.json = true;
        }
        i += 1;
    }

    b
}

fn rect_json(r: Rect) -> String {
    format!(
        "{{\"left\":{},\"top\":{},\"right\":{},\"bottom\":{}}}",
        r.left, r.top, r.right, r.bottom
    )
}

fn crop_rect_json(r: CropRect) -> String {
    format!("{{\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}", r.x, r.y, r.w, r.h)
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

fn error_json(err: &ErrorInfo) -> String {
    let mut s = format!(
        "{{\"message\":\"{}\",\"where\":\"{}\"",
        json_escape(&err.message),
        json_escape(&err.where_)
    );
    if let Some(hr) = err.hresult {
        s.push_str(&format!(",\"hresult\":\"{}\"", to_hex32(hr)));
    }
    if let Some(w) = err.win32_error {
        s.push_str(&format!(",\"win32_error\":{w}"));
    }
    s.push('}');
    s
}

fn windows_json_array(ws: &[WindowInfo]) -> String {
    let mut s = String::from("[");
    for (idx, w) in ws.iter().enumerate() {
        if idx > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"hwnd\":{},\"pid\":{},\"title\":\"{}\",\"class\":\"{}\",\"rect\":{},\"visible\":{},\"iconic\":{},\"cloaked\":{}}}",
            w.hwnd as u64,
            w.pid,
            json_escape(&w.title),
            json_escape(&w.class_name),
            rect_json(w.rect),
            w.visible,
            w.iconic,
            w.cloaked,
        ));
    }
    s.push(']');
    s
}

fn monitors_json_array(ms: &[MonitorInfo]) -> String {
    let mut s = String::from("[");
    for (idx, m) in ms.iter().enumerate() {
        if idx > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"index\":{},\"name\":\"{}\",\"desktop\":{},\"primary\":{}}}",
            m.index,
            json_escape(&m.name),
            rect_json(m.desktop),
            m.primary,
        ));
    }
    s.push(']');
    s
}

fn run_list_windows(parsed: &ParsedArgs) -> RunResult {
    let mut rr = RunResult::default();
    let ws = enumerate_windows();
    rr.ok = true;
    rr.exit_code = 0;
    rr.json = format!(
        "{{\"ok\":true,\"command\":\"list windows\",\"timestamp\":\"{}\",\"windows\":{}}}",
        iso8601_now_local(),
        windows_json_array(&ws)
    );

    if !parsed.common.json {
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
    rr.json = format!(
        "{{\"ok\":true,\"command\":\"list monitors\",\"timestamp\":\"{}\",\"monitors\":{}}}",
        iso8601_now_local(),
        monitors_json_array(&ms)
    );

    if !parsed.common.json {
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

fn run_cap(parsed: &ParsedArgs, logger: Option<&Logger>, dpi_applied: &str) -> RunResult {
    let mut rr = RunResult::default();
    let start = Instant::now();

    let windows = enumerate_windows();
    let monitors = enumerate_monitors();

    let mut ctx = CaptureContext {
        method: parsed.cap.method.clone(),
        cap: parsed.cap.clone(),
        common: parsed.common.clone(),
        window: None,
        monitor: None,
        capture_rect_screen: Rect::default(),
        logger,
    };

    let method = &parsed.cap.method;

    if parsed.cap.target == TargetType::Window
        || method.contains("window")
        || method.contains("printwindow")
        || method.contains("client")
        || method.contains("windowdc")
    {
        match resolve_window_target(&parsed.cap.window_query, &windows, logger) {
            Ok((w, reason)) => {
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
            Err(err) => {
                rr.err = err;
                rr.exit_code = 1;
                return rr;
            }
        }
    }

    if parsed.cap.target == TargetType::Screen
        || method.contains("monitor")
        || method == "dxgi-window"
    {
        if parsed.cap.screen_query.virtual_screen {
            ctx.capture_rect_screen = virtual_screen_rect();
        } else if let Some(token) = &parsed.cap.screen_query.monitor {
            match find_monitor_by_token(&monitors, token) {
                Some(mon) => {
                    ctx.capture_rect_screen = mon.desktop;
                    ctx.monitor = Some(mon);
                }
                None => {
                    rr.err = ErrorInfo::new("monitor not found", "RunCap");
                    rr.exit_code = 1;
                    return rr;
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

    let mut img: Option<ImageBuffer> = None;
    let mut cap_err = ErrorInfo::default();
    let mut adapter_index: i32 = -1;
    let mut output_index: i32 = -1;
    let mut cap_ok = false;

    for attempt in 0..=parsed.common.retry {
        let result: Result<ImageBuffer, ErrorInfo> = if parsed.cap.method.starts_with("gdi-") {
            capture_with_gdi(&ctx)
        } else if parsed.cap.method.starts_with("dxgi-") {
            match capture_with_dxgi(&ctx) {
                Ok((buf, a, o)) => {
                    adapter_index = a;
                    output_index = o;
                    Ok(buf)
                }
                Err(e) => Err(e),
            }
        } else if parsed.cap.method.starts_with("wgc-") {
            capture_with_wgc(&ctx)
        } else {
            Err(ErrorInfo::new("unknown method", "RunCap"))
        };

        match result {
            Ok(buf) => {
                img = Some(buf);
                cap_ok = true;
                break;
            }
            Err(e) => {
                cap_err = e;
                if let Some(lg) = logger {
                    lg.log(
                        LogLevel::Warn,
                        &format!(
                            "capture attempt failed attempt={} where={}",
                            attempt, cap_err.where_
                        ),
                    );
                }
            }
        }
    }

    if !cap_ok {
        rr.err = cap_err;
        rr.exit_code = 1;
        return rr;
    }

    let mut img = img.expect("cap_ok implies img is set");

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

    // resolve_crop_rect_screen never returns an invalid rect on the Ok path
    // (the C++ version signaled that case through the same error out-param),
    // so no separate validity check is needed here.
    let crop_rect = match resolve_crop_rect_screen(
        crop_mode,
        parsed.cap.crop_rect,
        ctx.window.as_ref(),
        img_rect,
        parsed.cap.pad,
    ) {
        Ok(r) => r,
        Err(e) => {
            rr.err = e;
            rr.exit_code = 1;
            return rr;
        }
    };

    if let Err(e) = crop_image_in_place(crop_rect, &mut img) {
        rr.err = e;
        rr.exit_code = 1;
        return rr;
    }

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

    if let Err(e) = save_png_wic(&img, &parsed.cap.out_path, parsed.common.overwrite) {
        rr.err = e;
        rr.exit_code = 1;
        return rr;
    }

    let duration_ms = start.elapsed().as_millis() as i32;

    let crop_out = CropRect {
        x: img.origin_x,
        y: img.origin_y,
        w: img.width,
        h: img.height,
    };

    let mut js = format!(
        "{{\"ok\":true,\"command\":\"cap\",\"method\":\"{}\",\"target\":\"{}\",\"out_path\":\"{}\",\"format\":\"png\",\"timestamp\":\"{}\",\"duration_ms\":{},\"dpi_mode\":\"{}\"",
        json_escape(&parsed.cap.method),
        cli::target_type_name(parsed.cap.target),
        json_escape(&parsed.cap.out_path),
        iso8601_now_local(),
        duration_ms,
        json_escape(dpi_applied),
    );

    if let Some(w) = &ctx.window {
        js.push_str(&format!(
            ",\"window\":{{\"hwnd\":{},\"pid\":{},\"title\":\"{}\",\"class\":\"{}\",\"rect\":{},\"client_rect_screen\":{},\"visible\":{},\"iconic\":{},\"cloaked\":{}}}",
            w.hwnd as u64,
            w.pid,
            json_escape(&w.title),
            json_escape(&w.class_name),
            rect_json(w.rect),
            rect_json(w.client_rect_screen),
            w.visible,
            w.iconic,
            w.cloaked,
        ));
    }

    if let Some(m) = &ctx.monitor {
        js.push_str(&format!(
            ",\"monitor\":{{\"index\":{},\"desktop\":{},\"primary\":{}}}",
            m.index,
            rect_json(m.desktop),
            m.primary,
        ));
    }

    js.push_str(&format!(
        ",\"crop\":{{\"mode\":\"{}\",\"rect\":{},\"pad\":{{\"l\":{},\"t\":{},\"r\":{},\"b\":{}}}}}",
        cli::crop_mode_name(crop_mode),
        crop_rect_json(crop_out),
        parsed.cap.pad.l,
        parsed.cap.pad.t,
        parsed.cap.pad.r,
        parsed.cap.pad.b,
    ));

    js.push_str(&format!(
        ",\"image_stats\":{{\"black_ratio\":{},\"transparent_ratio\":{},\"avg_luma\":{}}},\"error\":null}}",
        stats.black_ratio, stats.transparent_ratio, stats.avg_luma
    ));

    rr.ok = true;
    rr.exit_code = 0;
    rr.json = js;

    if let Some(lg) = logger {
        lg.log(
            LogLevel::Info,
            &format!(
                "result=success out_path={} duration_ms={}",
                parsed.cap.out_path, duration_ms
            ),
        );
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
        logger.log(LogLevel::Info, &format!("argv={}", p.raw_args.join(" ")));
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
    format!(
        "{{\"ok\":false,\"command\":\"{}\",\"method\":\"{}\",\"target\":\"{}\",\"out_path\":\"{}\",\"format\":\"png\",\"timestamp\":\"{}\",\"duration_ms\":{},\"dpi_mode\":\"{}\",\"window\":null,\"monitor\":null,\"crop\":null,\"image_stats\":null,\"error\":{}}}",
        json_escape(command),
        json_escape(method),
        json_escape(target),
        json_escape(out_path),
        iso8601_now_local(),
        duration_ms,
        json_escape(dpi_mode),
        error_json(err),
    )
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
    logger.init(&boot.log_dir, &boot.command, boot.log_level);

    let parsed = cli::parse_args(&argv);

    let requested_dpi = if parsed.ok {
        parsed.args.common.dpi_mode
    } else {
        DpiMode::PerMonitorV2
    };
    let dpi_applied = apply_dpi_mode(requested_dpi, Some(&logger));

    log_startup(
        Some(&logger),
        if parsed.ok { Some(&parsed.args) } else { None },
        &dpi_applied,
    );

    if !parsed.ok {
        logger.log(LogLevel::Error, &format!("parse error: {}", parsed.error));
        if boot.json {
            let err = ErrorInfo::new(parsed.error.clone(), "ParseArgs");
            println!(
                "{}",
                build_failure_json("unknown", "", "", "", &dpi_applied, 0, &err)
            );
        } else {
            eprintln!("Error: {}\n\n{}", parsed.error, cli::build_help_text());
        }
        return 2;
    }

    if parsed.show_help || parsed.args.command == CommandType::Help {
        print!("{}", cli::build_help_text());
        return 0;
    }

    let rr = if parsed.args.command == CommandType::ListWindows {
        run_list_windows(&parsed.args)
    } else if parsed.args.command == CommandType::ListMonitors {
        run_list_monitors(&parsed.args)
    } else if parsed.args.cap.hotkey_enabled {
        match wait_for_hotkey(&parsed.args, Some(&logger)) {
            Ok(()) => run_cap(&parsed.args, Some(&logger), &dpi_applied),
            Err(e) => RunResult {
                err: e,
                exit_code: 1,
                ..Default::default()
            },
        }
    } else {
        run_cap(&parsed.args, Some(&logger), &dpi_applied)
    };

    if rr.ok {
        logger.log(LogLevel::Info, "result=success");
        if parsed.args.common.json {
            println!("{}", rr.json);
        } else if parsed.args.command == CommandType::Cap {
            println!("ok: {}", parsed.args.cap.out_path);
        }
        return rr.exit_code;
    }

    logger.log(
        LogLevel::Error,
        &format!(
            "result=failure where={} message={}",
            rr.err.where_, rr.err.message
        ),
    );

    if parsed.args.common.json || parsed.args.command == CommandType::Cap {
        let command_str = if parsed.args.command == CommandType::Cap {
            "cap"
        } else {
            "list"
        };
        println!(
            "{}",
            build_failure_json(
                command_str,
                &parsed.args.cap.method,
                cli::target_type_name(parsed.args.cap.target),
                &parsed.args.cap.out_path,
                &dpi_applied,
                0,
                &rr.err,
            )
        );
    } else {
        eprintln!("Error: {} ({})", rr.err.message, rr.err.where_);
    }

    rr.exit_code
}
