//! Port of src/monitor_enum.cpp.

use crate::types::{MonitorInfo, Rect};
use crate::util::utf8_from_wide;

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

fn to_rect(r: RECT) -> Rect {
    Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

extern "system" fn enum_monitors_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let vec = unsafe { &mut *(lparam.0 as *mut Vec<MonitorInfo>) };

    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe { GetMonitorInfoW(hmon, &mut mi.monitorInfo as *mut _) };
    if !ok.as_bool() {
        return true.into();
    }

    let name_len = mi
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(mi.szDevice.len());

    let info = MonitorInfo {
        hmon: hmon.0 as isize,
        index: vec.len() as i32,
        name: utf8_from_wide(&mi.szDevice[..name_len]),
        desktop: to_rect(mi.monitorInfo.rcMonitor),
        primary: (mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
    };
    vec.push(info);
    true.into()
}

/// EnumDisplayMonitors; index is enumeration order.
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut out: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc),
            LPARAM(&mut out as *mut Vec<MonitorInfo> as isize),
        );
    }
    out
}

/// Mimics C++ `std::stoi`: skips leading whitespace, then parses an
/// optional sign followed by decimal digits, ignoring any trailing
/// characters. Returns `None` if no digits are present (std::stoi throws).
fn stoi_like(token: &str) -> Option<i32> {
    let trimmed = token.trim_start();
    let mut chars = trimmed.chars().peekable();
    let mut s = String::new();
    if let Some(&c) = chars.peek() {
        if c == '+' || c == '-' {
            s.push(c);
            chars.next();
        }
    }
    let mut has_digit = false;
    for c in chars {
        if c.is_ascii_digit() {
            has_digit = true;
            s.push(c);
        } else {
            break;
        }
    }
    if !has_digit {
        return None;
    }
    s.parse::<i32>().ok()
}

/// token is "primary" or a monitor index in decimal.
pub fn find_monitor_by_token(monitors: &[MonitorInfo], token: &str) -> Option<MonitorInfo> {
    if token == "primary" {
        return monitors.iter().find(|m| m.primary).cloned();
    }

    let idx = stoi_like(token)?;
    monitors.iter().find(|m| m.index == idx).cloned()
}
