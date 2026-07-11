use crate::types::MonitorInfo;
use crate::util::utf8_from_wide;

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

/// Enumeration state threaded through `LPARAM`. `next_index` is advanced for
/// every enumerated monitor, including ones where `GetMonitorInfoW` fails, so
/// a transient failure leaves a gap in `index` instead of shifting every
/// later monitor's index down by one.
struct EnumState {
    list: Vec<MonitorInfo>,
    next_index: i32,
}

extern "system" fn enum_monitors_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let state = unsafe { &mut *(lparam.0 as *mut EnumState) };
    let index = state.next_index;
    state.next_index += 1;

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
        index,
        name: utf8_from_wide(&mi.szDevice[..name_len]),
        desktop: mi.monitorInfo.rcMonitor.into(),
        primary: (mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
    };
    state.list.push(info);
    true.into()
}

/// EnumDisplayMonitors; index is enumeration order (including monitors
/// skipped due to a failed `GetMonitorInfoW`, so indices stay stable).
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut state = EnumState {
        list: Vec::new(),
        next_index: 0,
    };
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc),
            LPARAM(&mut state as *mut EnumState as isize),
        );
    }
    state.list
}

/// Parses a decimal monitor index token; returns `None` for non-numeric input.
fn parse_monitor_index(token: &str) -> Option<i32> {
    token.trim().parse::<i32>().ok()
}

/// token is "primary" or a monitor index in decimal.
pub fn find_monitor_by_token(monitors: &[MonitorInfo], token: &str) -> Option<MonitorInfo> {
    if token == "primary" {
        return monitors.iter().find(|m| m.primary).cloned();
    }

    let idx = parse_monitor_index(token)?;
    monitors.iter().find(|m| m.index == idx).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rect;

    fn monitor(index: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            hmon: 0x1000 + index as isize,
            index,
            name: format!("\\\\.\\DISPLAY{}", index + 1),
            desktop: Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            primary,
        }
    }

    fn sample_monitors() -> Vec<MonitorInfo> {
        vec![monitor(0, false), monitor(1, true), monitor(2, false)]
    }

    #[test]
    fn parse_monitor_index_reads_decimal_and_trims() {
        assert_eq!(parse_monitor_index("2"), Some(2));
        assert_eq!(parse_monitor_index("  1 "), Some(1));
        assert_eq!(parse_monitor_index("primary"), None);
        assert_eq!(parse_monitor_index("1x"), None);
    }

    #[test]
    fn find_monitor_by_token_primary() {
        let m = find_monitor_by_token(&sample_monitors(), "primary").unwrap();
        assert_eq!(m.index, 1);
        assert!(m.primary);
    }

    #[test]
    fn find_monitor_by_token_valid_index() {
        let m = find_monitor_by_token(&sample_monitors(), "2").unwrap();
        assert_eq!(m.index, 2);
    }

    #[test]
    fn find_monitor_by_token_out_of_range_and_garbage() {
        assert!(find_monitor_by_token(&sample_monitors(), "9").is_none());
        assert!(find_monitor_by_token(&sample_monitors(), "nope").is_none());
    }
}
