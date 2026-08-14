use crate::logging::Logger;
use crate::types::{ErrorInfo, LogLevel, Rect, TargetWindowQuery, WindowInfo};
use crate::util::utf8_from_wide;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOT, GetAncestor, GetClassNameW, GetClientRect, GetForegroundWindow,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible,
};

/// Reads the window title as UTF-8.
pub fn get_window_text_utf8(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut ws: Vec<u16> = vec![0u16; len + 1];
    // Do not trust GetWindowTextLengthW alone: GetWindowTextW may copy fewer
    // code units; slice by the returned length so trailing NULs stay out of the title.
    let copied = if len > 0 {
        unsafe { GetWindowTextW(hwnd, &mut ws) }.max(0) as usize
    } else {
        0
    };
    utf8_from_wide(&ws[..copied])
}

/// Reads the window class name as UTF-8.
fn get_class_name_utf8(hwnd: HWND) -> String {
    let mut buf = [0u16; 257];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    let len = len.max(0) as usize;
    utf8_from_wide(&buf[..len])
}

/// Returns the client area converted to screen coordinates.
fn get_client_rect_screen(hwnd: HWND) -> Rect {
    let mut cr = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut cr) }.is_err() {
        return Rect::default();
    }
    let mut points = [
        POINT {
            x: cr.left,
            y: cr.top,
        },
        POINT {
            x: cr.right,
            y: cr.bottom,
        },
    ];
    // Do not use ClientToScreen on individual corners: RTL layout mirrors x
    // and can yield right < left. MapWindowPoints maps both corners together.
    unsafe {
        MapWindowPoints(Some(hwnd), None, &mut points);
    }
    let (mut left, mut right) = (points[0].x, points[1].x);
    if left > right {
        std::mem::swap(&mut left, &mut right);
    }
    let (mut top, mut bottom) = (points[0].y, points[1].y);
    if top > bottom {
        std::mem::swap(&mut top, &mut bottom);
    }
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

/// Returns DWM extended frame bounds, or `fallback` when DWM is unavailable.
fn get_dwm_frame_rect(hwnd: HWND, fallback: Rect) -> Rect {
    let mut r = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if ok.is_ok() { r.into() } else { fallback }
}

fn area(r: &Rect) -> i64 {
    (r.width().max(0) as i64) * (r.height().max(0) as i64)
}

/// ASCII case-insensitive substring match for `--title` filters.
fn contains_i(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    // Do not call str::windows with an empty needle: it panics. Empty needle
    // means "match all" for title/class filters.
    hay.as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// `EnumWindows` callback: appends one [`WindowInfo`] per top-level HWND into
/// the `Vec` in `LPARAM`.
extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let vec = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };

    let mut w = WindowInfo {
        hwnd: hwnd.0 as isize,
        pid: 0,
        title: String::new(),
        class_name: String::new(),
        rect: Rect::default(),
        client_rect_screen: Rect::default(),
        dwm_frame_rect: Rect::default(),
        visible: false,
        iconic: false,
        cloaked: false,
    };

    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut w.pid)) };
    w.title = get_window_text_utf8(hwnd);
    w.class_name = get_class_name_utf8(hwnd);
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    w.rect = r.into();
    w.client_rect_screen = get_client_rect_screen(hwnd);
    w.dwm_frame_rect = get_dwm_frame_rect(hwnd, w.rect);
    w.visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    w.iconic = unsafe { IsIconic(hwnd) }.as_bool();
    let mut cloaked: u32 = 0;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        )
    };
    if ok.is_ok() {
        w.cloaked = cloaked != 0;
    }

    vec.push(w);
    true.into()
}

/// EnumWindows over all top-level windows, filling every WindowInfo field
/// (title/class as UTF-8, rects, visible/iconic/cloaked via DWM).
pub fn enumerate_windows() -> Vec<WindowInfo> {
    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut out as *mut Vec<WindowInfo> as isize),
        );
    }
    out
}

/// Resolve the target window. Priority: --hwnd exact match, then
/// --foreground, then pid/title(case-insensitive substring)/class(exact)
/// filters ranked by (visible&&!iconic&&!cloaked, is-root, area) descending.
/// Returns the window and the human-readable match reason.
pub fn resolve_window_target(
    query: &TargetWindowQuery,
    all: &[WindowInfo],
    logger: &Logger,
) -> Result<(WindowInfo, String), ErrorInfo> {
    if let Some(hwnd_val) = query.hwnd {
        let target = hwnd_val as isize;
        for w in all {
            if w.hwnd == target {
                return Ok((w.clone(), "matched by --hwnd".to_string()));
            }
        }
        return Err(ErrorInfo::new(
            "window not found by --hwnd",
            "ResolveWindowTarget",
        ));
    }

    if query.foreground {
        let fg = unsafe { GetForegroundWindow() };
        for w in all {
            if w.hwnd == fg.0 as isize {
                return Ok((w.clone(), "matched by --foreground".to_string()));
            }
        }
        return Err(ErrorInfo::new(
            "foreground window not found",
            "ResolveWindowTarget",
        ));
    }

    let mut candidates: Vec<&WindowInfo> = Vec::new();
    for w in all {
        if let Some(pid) = query.pid
            && w.pid as i32 != pid
        {
            continue;
        }
        if let Some(title) = &query.title
            && !contains_i(&w.title, title)
        {
            continue;
        }
        if let Some(class_name) = &query.class_name
            && &w.class_name != class_name
        {
            continue;
        }
        candidates.push(w);
    }

    if candidates.is_empty() {
        return Err(ErrorInfo::new("no matching windows", "ResolveWindowTarget"));
    }

    let rank = |w: &WindowInfo| -> (bool, bool, i64) {
        let usable = w.visible && !w.iconic && !w.cloaked;
        let is_root = unsafe { GetAncestor(HWND(w.hwnd as *mut _), GA_ROOT) }.0 as isize == w.hwnd;
        (usable, is_root, area(&w.rect))
    };

    // Do not call GetAncestor inside sort_by: rank each candidate once instead.
    candidates.sort_by_cached_key(|w| std::cmp::Reverse(rank(w)));

    let winner = candidates[0].clone();
    let reason =
        "matched by filters, selected by priority(visible&&!iconic&&!cloaked > root > max area)"
            .to_string();

    logger.log(
        LogLevel::Info,
        &format!("ResolveWindowTarget candidates={}", candidates.len()),
    );

    Ok((winner, reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_i_matches_ignoring_case() {
        assert!(contains_i("My Notepad Window", "notepad"));
        assert!(contains_i("NOTEPAD", "Notepad"));
    }

    #[test]
    fn contains_i_non_match_returns_false() {
        assert!(!contains_i("My Notepad Window", "chrome"));
    }

    #[test]
    fn contains_i_empty_needle_matches_without_panicking() {
        assert!(contains_i("anything", ""));
        assert!(contains_i("", ""));
    }
}
