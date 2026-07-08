//! Port of src/cli.cpp: hand-rolled arg parser (kept bespoke so behavior and
//! error messages match the C++ CLI exactly).

use screencap_core::types::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_F1, VK_SNAPSHOT, VK_SPACE,
};

#[derive(Clone, Debug)]
pub struct ParsedArgs {
    pub command: CommandType,
    pub common: CommonOptions,
    pub cap: CapOptions,
    pub raw_args: Vec<String>,
}

impl Default for ParsedArgs {
    fn default() -> Self {
        Self {
            command: CommandType::Help,
            common: CommonOptions::default(),
            cap: CapOptions::default(),
            raw_args: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParseResult {
    pub ok: bool,
    pub show_help: bool,
    pub args: ParsedArgs,
    pub error: String,
}

// ---------------------------------------------------------------------------
// small parsing helpers (mirror the free functions in the anonymous namespace
// of src/cli.cpp)
// ---------------------------------------------------------------------------

fn need_value(i: usize, argc: usize, name: &str, err: &mut String) -> bool {
    if i + 1 >= argc {
        *err = format!("missing value for {name}");
        return false;
    }
    true
}

/// Mirrors `strtol` used by the C++ ParseInt: optional leading whitespace,
/// optional sign, then digits, and the whole remainder must be consumed.
fn parse_int(s: &str) -> Option<i32> {
    let trimmed = s.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut idx = 0usize;
    if bytes[idx] == b'+' || bytes[idx] == b'-' {
        idx += 1;
    }
    let digits_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == digits_start || idx != bytes.len() {
        return None;
    }
    trimmed.parse::<i32>().ok()
}

fn parse_u64(s: &str) -> Option<u64> {
    let trimmed = s.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut idx = 0usize;
    if bytes[idx] == b'+' {
        idx += 1;
    }
    let digits_start = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx == digits_start || idx != bytes.len() {
        return None;
    }
    trimmed.parse::<u64>().ok()
}

fn parse_dpi_mode(s: &str) -> DpiMode {
    match s {
        "auto" => DpiMode::Auto,
        "system" => DpiMode::System,
        _ => DpiMode::PerMonitorV2,
    }
}

fn parse_crop_mode(s: &str) -> CropMode {
    match s {
        "window" => CropMode::Window,
        "client" => CropMode::Client,
        "dwm-frame" => CropMode::DwmFrame,
        "manual" => CropMode::Manual,
        _ => CropMode::None,
    }
}

fn to_lower_ascii(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_lowercase()).collect()
}

fn parse_function_key(token: &str) -> Option<u32> {
    if token.len() < 2 || !token.starts_with('f') {
        return None;
    }
    let n = parse_int(&token[1..])?;
    if !(1..=24).contains(&n) {
        return None;
    }
    Some(VK_F1.0 as u32 + (n - 1) as u32)
}

/// Mirrors ParseHotkey's `std::getline(iss, token, '+')` splitting: a
/// trailing '+' does not produce a spurious empty final token (the stream is
/// already at eof at that point), but any other empty token (leading or
/// consecutive '+') is rejected, just like the C++ version.
fn parse_hotkey(spec: &str) -> Option<(u32, u32)> {
    let mut mods: u32 = MOD_NOREPEAT.0;
    let mut vk: u32 = 0;
    let mut has_modifier = false;

    let mut tokens: Vec<&str> = spec.split('+').collect();
    if spec.ends_with('+') {
        tokens.pop();
    }

    for raw in tokens {
        let token = to_lower_ascii(raw);
        if token.is_empty() {
            return None;
        }

        match token.as_str() {
            "ctrl" | "control" => {
                mods |= MOD_CONTROL.0;
                has_modifier = true;
                continue;
            }
            "alt" => {
                mods |= MOD_ALT.0;
                has_modifier = true;
                continue;
            }
            "shift" => {
                mods |= MOD_SHIFT.0;
                has_modifier = true;
                continue;
            }
            "win" | "windows" => {
                mods |= MOD_WIN.0;
                has_modifier = true;
                continue;
            }
            _ => {}
        }

        if vk != 0 {
            return None;
        }

        if token.chars().count() == 1 {
            let c = token.as_bytes()[0];
            if c.is_ascii_lowercase() {
                vk = (b'A' as u32) + (c - b'a') as u32;
                continue;
            }
            if c.is_ascii_digit() {
                vk = c as u32;
                continue;
            }
            return None;
        }

        if let Some(v) = parse_function_key(&token) {
            vk = v;
            continue;
        }

        match token.as_str() {
            "printscreen" | "prtsc" | "snapshot" => {
                vk = VK_SNAPSHOT.0 as u32;
                continue;
            }
            "space" => {
                vk = VK_SPACE.0 as u32;
                continue;
            }
            _ => {}
        }

        return None;
    }

    if has_modifier && vk != 0 {
        Some((mods, vk))
    } else {
        None
    }
}

/// Full CLI grammar of the C++ ParseArgs, including validation
/// (cap needs --method/--out, window target needs a query, --format png only,
/// manual crop needs --crop-rect, --hotkey-foreground needs --hotkey, hotkey
/// spec parsing like ctrl+shift+s / alt+f9).
pub fn parse_args(argv: &[String]) -> ParseResult {
    let argc = argv.len();
    let mut r = ParseResult::default();

    if argc <= 1 {
        r.show_help = true;
        r.ok = true;
        return r;
    }

    let mut out = ParsedArgs {
        raw_args: argv.to_vec(),
        ..Default::default()
    };

    let mut i = 1usize;
    let cmd = argv[i].clone();
    i += 1;

    if cmd == "cap" {
        out.command = CommandType::Cap;
    } else if cmd == "list" {
        if i >= argc {
            r.error = "list needs subcommand: windows|monitors".to_string();
            return r;
        }
        let sub = argv[i].clone();
        i += 1;
        if sub == "windows" {
            out.command = CommandType::ListWindows;
        } else if sub == "monitors" {
            out.command = CommandType::ListMonitors;
        } else {
            r.error = format!("unknown list subcommand: {sub}");
            return r;
        }
    } else if cmd == "-h" || cmd == "--help" || cmd == "help" {
        r.show_help = true;
        r.ok = true;
        return r;
    } else {
        r.error = format!("unknown command: {cmd}");
        return r;
    }

    while i < argc {
        let a = argv[i].clone();
        let is_cap = out.command == CommandType::Cap;

        if a == "--log-dir" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.common.log_dir = argv[i].clone();
        } else if a == "--log-level" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.common.log_level = screencap_core::logging::parse_log_level(&argv[i]);
        } else if a == "--json" {
            out.common.json = true;
        } else if a == "--timeout-ms" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            match parse_int(&argv[i]) {
                Some(v) => out.common.timeout_ms = v,
                None => {
                    r.error = "invalid --timeout-ms".to_string();
                    return r;
                }
            }
        } else if a == "--retry" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            match parse_int(&argv[i]) {
                Some(v) => out.common.retry = v,
                None => {
                    r.error = "invalid --retry".to_string();
                    return r;
                }
            }
        } else if a == "--overwrite" {
            out.common.overwrite = true;
        } else if a == "--dpi-mode" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.common.dpi_mode = parse_dpi_mode(&argv[i]);
        } else if is_cap && a == "--method" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.method = argv[i].clone();
        } else if is_cap && a == "--target" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            let v = argv[i].clone();
            if v == "window" {
                out.cap.target = TargetType::Window;
            } else if v == "screen" {
                out.cap.target = TargetType::Screen;
            } else {
                r.error = "invalid --target".to_string();
                return r;
            }
        } else if is_cap && a == "--out" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.out_path = argv[i].clone();
        } else if is_cap && a == "--stdout" {
            r.error = "--stdout is not supported in this version".to_string();
            return r;
        } else if is_cap && a == "--hwnd" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            match parse_u64(&argv[i]) {
                Some(v) => out.cap.window_query.hwnd = Some(v),
                None => {
                    r.error = "invalid --hwnd".to_string();
                    return r;
                }
            }
        } else if is_cap && a == "--pid" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            match parse_int(&argv[i]) {
                Some(v) => out.cap.window_query.pid = Some(v),
                None => {
                    r.error = "invalid --pid".to_string();
                    return r;
                }
            }
        } else if is_cap && a == "--foreground" {
            out.cap.window_query.foreground = true;
        } else if is_cap && a == "--title" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.window_query.title = Some(argv[i].clone());
        } else if is_cap && a == "--class" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.window_query.class_name = Some(argv[i].clone());
        } else if is_cap && a == "--monitor" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.screen_query.monitor = Some(argv[i].clone());
        } else if is_cap && a == "--virtual-screen" {
            out.cap.screen_query.virtual_screen = true;
        } else if is_cap && a == "--crop" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.crop_mode = parse_crop_mode(&argv[i]);
        } else if is_cap && a == "--crop-rect" {
            if i + 4 >= argc {
                r.error = "--crop-rect needs 4 values".to_string();
                return r;
            }
            let x = parse_int(&argv[i + 1]);
            let y = parse_int(&argv[i + 2]);
            let w = parse_int(&argv[i + 3]);
            let h = parse_int(&argv[i + 4]);
            match (x, y, w, h) {
                (Some(x), Some(y), Some(w), Some(h)) => {
                    out.cap.crop_rect = Some(CropRect { x, y, w, h });
                    i += 4;
                }
                _ => {
                    r.error = "invalid --crop-rect".to_string();
                    return r;
                }
            }
        } else if is_cap && a == "--pad" {
            if i + 4 >= argc {
                r.error = "--pad needs 4 values".to_string();
                return r;
            }
            let l = parse_int(&argv[i + 1]);
            let t = parse_int(&argv[i + 2]);
            let rr = parse_int(&argv[i + 3]);
            let b = parse_int(&argv[i + 4]);
            match (l, t, rr, b) {
                (Some(l), Some(t), Some(rr), Some(b)) => {
                    out.cap.pad = Pad { l, t, r: rr, b };
                    i += 4;
                }
                _ => {
                    r.error = "invalid --pad".to_string();
                    return r;
                }
            }
        } else if is_cap && a == "--format" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.format = argv[i].clone();
        } else if is_cap && a == "--force-alpha" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            match parse_int(&argv[i]) {
                Some(255) => out.cap.force_alpha_255 = true,
                _ => {
                    r.error = "--force-alpha only supports 255".to_string();
                    return r;
                }
            }
        } else if is_cap && a == "--hotkey" {
            if !need_value(i, argc, &a, &mut r.error) {
                return r;
            }
            i += 1;
            out.cap.hotkey_spec = argv[i].clone();
            match parse_hotkey(&out.cap.hotkey_spec) {
                Some((mods, vk)) => {
                    out.cap.hotkey_modifiers = mods;
                    out.cap.hotkey_vk = vk;
                }
                None => {
                    r.error = "invalid --hotkey (ex: ctrl+shift+s, alt+f9)".to_string();
                    return r;
                }
            }
            out.cap.hotkey_enabled = true;
        } else if is_cap && a == "--hotkey-foreground" {
            out.cap.hotkey_foreground = true;
            out.cap.window_query.foreground = true;
        } else {
            r.error = format!("unknown option: {a}");
            return r;
        }

        i += 1;
    }

    if out.command == CommandType::Cap {
        if out.cap.method.is_empty() {
            r.error = "cap needs --method".to_string();
            return r;
        }
        if out.cap.out_path.is_empty() {
            r.error = "cap needs --out".to_string();
            return r;
        }
        if out.cap.format != "png" {
            r.error = "only --format png is supported".to_string();
            return r;
        }
        if out.cap.target == TargetType::Window {
            let has_window_target = out.cap.window_query.hwnd.is_some()
                || out.cap.window_query.pid.is_some()
                || out.cap.window_query.foreground
                || out.cap.window_query.title.is_some()
                || out.cap.window_query.class_name.is_some();
            if !has_window_target {
                r.error = "window target needs one of --hwnd/--pid/--foreground/--title/--class"
                    .to_string();
                return r;
            }
        } else if out.cap.screen_query.monitor.is_none() && !out.cap.screen_query.virtual_screen {
            r.error = "screen target needs --monitor or --virtual-screen".to_string();
            return r;
        }
        if out.cap.crop_mode == CropMode::Manual && out.cap.crop_rect.is_none() {
            r.error = "manual crop needs --crop-rect".to_string();
            return r;
        }
        if out.cap.hotkey_foreground && !out.cap.hotkey_enabled {
            r.error = "--hotkey-foreground needs --hotkey".to_string();
            return r;
        }
    }

    r.ok = true;
    r.args = out;
    r
}

/// Not exercised by the run flow (mirrors DpiModeName, which src/main.cpp
/// never calls either), kept as part of the frozen public surface.
#[allow(dead_code)]
pub fn dpi_mode_name(mode: DpiMode) -> &'static str {
    match mode {
        DpiMode::Auto => "auto",
        DpiMode::PerMonitorV2 => "per-monitor-v2",
        DpiMode::System => "system",
    }
}

pub fn target_type_name(t: TargetType) -> &'static str {
    match t {
        TargetType::Window => "window",
        TargetType::Screen => "screen",
    }
}

pub fn crop_mode_name(m: CropMode) -> &'static str {
    match m {
        CropMode::None => "none",
        CropMode::Window => "window",
        CropMode::Client => "client",
        CropMode::DwmFrame => "dwm-frame",
        CropMode::Manual => "manual",
    }
}

pub fn build_help_text() -> String {
    concat!(
        "screencap-cli - Windows screenshot comparison CLI\n\n",
        "Commands:\n",
        "  cap\n",
        "  list windows\n",
        "  list monitors\n\n",
        "Examples:\n",
        "  screencap-cli list windows --json\n",
        "  screencap-cli cap --method dxgi-monitor --target screen --monitor primary --out a.png\n",
        "  screencap-cli cap --method wgc-window --target window --foreground --hotkey ctrl+shift+s --out a.png\n",
    )
    .to_string()
}
