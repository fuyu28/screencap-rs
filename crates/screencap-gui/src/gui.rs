//! Win32 window-picker GUI. Lists capturable windows in a ListView, lets the
//! user pick an output path, and shells out to screencap-cli.exe (next to this
//! exe) to do the actual WGC window capture.

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{UpdateWindow, COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, LVCF_TEXT, LVCF_WIDTH,
    LVCOLUMNW, LVIF_PARAM, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS, LVM_GETITEMW, LVM_GETNEXTITEM,
    LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMTEXTW,
    LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_EX_GRIDLINES, LVS_REPORT,
    LVS_SHOWSELALWAYS, LVS_SINGLESEL, NMHDR, NM_DBLCLK, WC_LISTVIEWW,
};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetAncestor, GetClientRect, GetMessageW,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, LoadCursorW, MessageBoxW, MoveWindow,
    PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, ES_AUTOHSCROLL, GA_ROOT, GWLP_USERDATA, HMENU,
    IDC_ARROW, MB_ICONERROR, MB_ICONINFORMATION, MSG, SW_SHOW, WINDOW_STYLE, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_NCCREATE, WM_NOTIFY, WM_SIZE, WNDCLASSW, WS_CHILD, WS_EX_CLIENTEDGE,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use screencap_core::types::WindowInfo;
use screencap_core::util::{build_timestamp_for_filename, utf8_from_wide, wide_from_utf8};
use screencap_core::window_enum::enumerate_windows;

const ID_LIST: u16 = 1001;
const ID_REFRESH: u16 = 1002;
const ID_OUT: u16 = 1004;
const ID_BROWSE: u16 = 1005;
const ID_CAPTURE: u16 = 1006;
const ID_STATUS: u16 = 1007;

/// Per-window state. A pointer to this struct is stored in GWLP_USERDATA so the
/// window procedure can recover its context.
#[derive(Default)]
struct GuiState {
    hwnd: HWND,
    list: HWND,
    refresh: HWND,
    out: HWND,
    browse: HWND,
    capture: HWND,
    status: HWND,
    windows: Vec<WindowInfo>,
}

fn control_id(id: u16) -> HMENU {
    HMENU(id as isize as *mut c_void)
}

fn to_wide(s: &str) -> Vec<u16> {
    let mut v = wide_from_utf8(s);
    v.push(0);
    v
}

fn to_wide_fixed(s: &str, len: usize) -> Vec<u16> {
    let mut v = wide_from_utf8(s);
    if v.len() > len - 1 {
        v.truncate(len - 1);
    }
    v.resize(len, 0);
    v
}

fn set_window_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, &HSTRING::from(text));
    }
}

fn get_window_text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buf) };
    buf.truncate(copied.max(0) as usize);
    utf8_from_wide(&buf)
}

fn set_status(state: &GuiState, text: &str) {
    set_window_text(state.status, text);
}

fn default_output_path() -> String {
    let filename = format!("screenshot_{}.png", build_timestamp_for_filename());
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(filename).to_string_lossy().into_owned(),
        Err(_) => filename,
    }
}

fn resize_controls(state: &GuiState) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(state.hwnd, &mut rc);
    }
    let pad = 10;
    let button_h = 28;
    let out_h = 24;
    let status_h = 22;
    let browse_w = 80;
    let capture_w = 92;
    let refresh_w = 80;
    let width = rc.right - rc.left;
    let height = rc.bottom - rc.top;

    unsafe {
        let _ = MoveWindow(state.refresh, pad, pad, refresh_w, button_h, true);
        let _ = MoveWindow(
            state.capture,
            width - pad - capture_w,
            pad,
            capture_w,
            button_h,
            true,
        );

        let out_y = pad + button_h + pad;
        let _ = MoveWindow(
            state.out,
            pad,
            out_y,
            width - pad * 3 - browse_w,
            out_h,
            true,
        );
        let _ = MoveWindow(
            state.browse,
            width - pad - browse_w,
            out_y,
            browse_w,
            out_h,
            true,
        );

        let list_y = out_y + out_h + pad;
        let list_h = height - list_y - status_h - pad * 2;
        let _ = MoveWindow(
            state.list,
            pad,
            list_y,
            width - pad * 2,
            list_h.max(80),
            true,
        );
        let _ = MoveWindow(
            state.status,
            pad,
            height - pad - status_h,
            width - pad * 2,
            status_h,
            true,
        );
    }
}

fn init_list_columns(list: HWND) {
    let columns: [(&str, i32); 4] = [("Title", 360), ("Class", 170), ("PID", 80), ("Rect", 180)];
    for (i, (text, width)) in columns.iter().enumerate() {
        let mut wtext = to_wide(text);
        let col = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            pszText: PWSTR(wtext.as_mut_ptr()),
            cx: *width,
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(i)),
                Some(LPARAM(&col as *const LVCOLUMNW as isize)),
            );
        }
    }
}

fn set_item_text(list: HWND, item: i32, sub_item: i32, text: &str) {
    let mut wtext = to_wide(text);
    let lv = LVITEMW {
        iSubItem: sub_item,
        pszText: PWSTR(wtext.as_mut_ptr()),
        cchTextMax: -1,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMTEXTW,
            Some(WPARAM(item as usize)),
            Some(LPARAM(&lv as *const LVITEMW as isize)),
        );
    }
}

fn is_pickable(w: &WindowInfo) -> bool {
    if !w.visible || w.iconic || w.cloaked || w.title.is_empty() {
        return false;
    }
    if !w.rect.is_valid() {
        return false;
    }
    let hwnd = HWND(w.hwnd as *mut c_void);
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    root == hwnd
}

fn refresh_windows(state: &mut GuiState) {
    state.windows.clear();
    unsafe {
        SendMessageW(
            state.list,
            LVM_DELETEALLITEMS,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
    }

    let mut pickable: Vec<WindowInfo> = enumerate_windows()
        .into_iter()
        .filter(is_pickable)
        .collect();
    pickable.sort_by(|a, b| a.title.cmp(&b.title));
    state.windows = pickable;

    for (i, w) in state.windows.iter().enumerate() {
        let mut title_w = to_wide(&w.title);
        let item = LVITEMW {
            mask: LVIF_TEXT | LVIF_PARAM,
            iItem: i as i32,
            pszText: PWSTR(title_w.as_mut_ptr()),
            lParam: LPARAM(i as isize),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                state.list,
                LVM_INSERTITEMW,
                Some(WPARAM(0)),
                Some(LPARAM(&item as *const LVITEMW as isize)),
            );
        }
        set_item_text(state.list, i as i32, 1, &w.class_name);
        set_item_text(state.list, i as i32, 2, &w.pid.to_string());
        let rect = format!(
            "{},{} {}x{}",
            w.rect.left,
            w.rect.top,
            w.rect.width(),
            w.rect.height()
        );
        set_item_text(state.list, i as i32, 3, &rect);
    }

    set_status(state, &format!("Windows: {}", state.windows.len()));
}

fn build_save_filter() -> Vec<u16> {
    let mut buf = Vec::new();
    for part in ["PNG image (*.png)", "*.png", "All files (*.*)", "*.*"] {
        buf.extend(wide_from_utf8(part));
        buf.push(0);
    }
    buf.push(0);
    buf
}

fn browse_output(state: &mut GuiState) {
    let current = get_window_text(state.out);
    let mut file_buf = to_wide_fixed(&current, 260);
    let filter = build_save_filter();

    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: state.hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrDefExt: w!("png"),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        ..Default::default()
    };

    if unsafe { GetSaveFileNameW(&mut ofn) }.as_bool() {
        let end = file_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(file_buf.len());
        let path = utf8_from_wide(&file_buf[..end]);
        set_window_text(state.out, &path);
    }
}

fn selected_window_index(state: &GuiState) -> Option<usize> {
    let item = unsafe {
        SendMessageW(
            state.list,
            LVM_GETNEXTITEM,
            Some(WPARAM(((-1i32) as isize) as usize)),
            Some(LPARAM(LVNI_SELECTED as isize)),
        )
    }
    .0 as i32;
    if item < 0 {
        return None;
    }
    let mut lv = LVITEMW {
        mask: LVIF_PARAM,
        iItem: item,
        ..Default::default()
    };
    let ok = unsafe {
        SendMessageW(
            state.list,
            LVM_GETITEMW,
            Some(WPARAM(0)),
            Some(LPARAM(&mut lv as *mut LVITEMW as isize)),
        )
    }
    .0;
    if ok == 0 {
        return None;
    }
    Some(lv.lParam.0 as usize)
}

fn cli_exe_path() -> PathBuf {
    let mut buf = vec![0u16; 32768];
    let len = unsafe { GetModuleFileNameW(None, &mut buf) };
    buf.truncate(len as usize);
    let exe = PathBuf::from(utf8_from_wide(&buf));
    exe.parent()
        .map(|p| p.join("screencap-cli.exe"))
        .unwrap_or_else(|| PathBuf::from("screencap-cli.exe"))
}

/// Shell out to screencap-cli.exe with `CREATE_NO_WINDOW` so no console flashes
/// up.
fn run_capture_process(window: &WindowInfo, out_path: &str) -> Result<(), String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let cli_path = cli_exe_path();
    if !cli_path.exists() {
        return Err("screencap-cli.exe was not found next to screencap.exe.".to_string());
    }

    let status = Command::new(&cli_path)
        .arg("cap")
        .arg("--target")
        .arg("window")
        .arg("--hwnd")
        .arg((window.hwnd as usize).to_string())
        .arg("--out")
        .arg(out_path)
        .arg("--overwrite")
        .arg("--json")
        .arg("--timeout-ms")
        .arg("2000")
        .arg("--force-alpha")
        .arg("255")
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "Capture failed. Exit code: {}",
            status.code().unwrap_or(1)
        )),
        Err(e) => Err(format!("Failed to start screencap-cli.exe: {e}")),
    }
}

fn capture_selected(state: &mut GuiState) {
    let idx = match selected_window_index(state) {
        Some(idx) if idx < state.windows.len() => idx,
        _ => {
            unsafe {
                MessageBoxW(
                    Some(state.hwnd),
                    w!("Select a window first."),
                    w!("screencap"),
                    MB_ICONINFORMATION,
                );
            }
            return;
        }
    };

    let out_path = get_window_text(state.out);
    if out_path.is_empty() {
        unsafe {
            MessageBoxW(
                Some(state.hwnd),
                w!("Choose an output path first."),
                w!("screencap"),
                MB_ICONINFORMATION,
            );
        }
        return;
    }

    unsafe {
        let _ = EnableWindow(state.capture, false);
    }
    set_status(state, "Capturing...");
    unsafe {
        let _ = UpdateWindow(state.hwnd);
    }

    let window = state.windows[idx].clone();
    let result = run_capture_process(&window, &out_path);

    unsafe {
        let _ = EnableWindow(state.capture, true);
    }

    match result {
        Ok(()) => set_status(state, &format!("Saved: {out_path}")),
        Err(err) => {
            set_status(state, &err);
            unsafe {
                MessageBoxW(
                    Some(state.hwnd),
                    &HSTRING::from(err.as_str()),
                    w!("screencap"),
                    MB_ICONERROR,
                );
            }
        }
    }
}

fn create_controls(state: &mut GuiState, hwnd: HWND) {
    state.hwnd = hwnd;
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map(|h| HINSTANCE(h.0))
        .unwrap_or_default();

    state.refresh = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("BUTTON"),
            w!("Refresh"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(control_id(ID_REFRESH)),
            Some(instance),
            None,
        )
    }
    .unwrap_or_default();

    state.out = unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(control_id(ID_OUT)),
            Some(instance),
            None,
        )
    }
    .unwrap_or_default();
    set_window_text(state.out, &default_output_path());

    state.browse = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("BUTTON"),
            w!("Browse"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(control_id(ID_BROWSE)),
            Some(instance),
            None,
        )
    }
    .unwrap_or_default();

    state.capture = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("BUTTON"),
            w!("Capture"),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(control_id(ID_CAPTURE)),
            Some(instance),
            None,
        )
    }
    .unwrap_or_default();

    state.list = unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            WC_LISTVIEWW,
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS),
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(control_id(ID_LIST)),
            Some(instance),
            None,
        )
    }
    .unwrap_or_default();
    unsafe {
        SendMessageW(
            state.list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            Some(WPARAM(0)),
            Some(LPARAM(
                (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_GRIDLINES) as isize,
            )),
        );
    }
    init_list_columns(state.list);

    state.status = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(control_id(ID_STATUS)),
            Some(instance),
            None,
        )
    }
    .unwrap_or_default();

    resize_controls(state);
    refresh_windows(state);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = lparam.0 as *const CREATESTRUCTW;
        let params = unsafe { (*cs).lpCreateParams };
        unsafe {
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, params as isize);
        }
        return LRESULT(1);
    }

    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut GuiState;

    match msg {
        WM_CREATE => {
            if let Some(state) = unsafe { state_ptr.as_mut() } {
                create_controls(state, hwnd);
            }
            return LRESULT(0);
        }
        WM_SIZE => {
            if let Some(state) = unsafe { state_ptr.as_mut() } {
                resize_controls(state);
            }
            return LRESULT(0);
        }
        WM_COMMAND => {
            if let Some(state) = unsafe { state_ptr.as_mut() } {
                let id = wparam.0 as u16;
                if id == ID_REFRESH {
                    refresh_windows(state);
                    return LRESULT(0);
                } else if id == ID_BROWSE {
                    browse_output(state);
                    return LRESULT(0);
                } else if id == ID_CAPTURE {
                    capture_selected(state);
                    return LRESULT(0);
                }
            }
        }
        WM_NOTIFY => {
            if let Some(state) = unsafe { state_ptr.as_mut() } {
                let hdr = lparam.0 as *const NMHDR;
                let (id_from, code) = unsafe { ((*hdr).idFrom, (*hdr).code) };
                if id_from == ID_LIST as usize && code == NM_DBLCLK {
                    capture_selected(state);
                    return LRESULT(0);
                }
            }
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            return LRESULT(0);
        }
        _ => {}
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Runs the GUI message loop; returns the process exit code.
pub fn run_gui() -> i32 {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);

        let instance = GetModuleHandleW(PCWSTR::null())
            .map(|h| HINSTANCE(h.0))
            .unwrap_or_default();

        let class_name = w!("ScreencapWindowPicker");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 as isize + 1) as *mut c_void),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let mut state = GuiState::default();
        let hwnd = match CreateWindowExW(
            Default::default(),
            class_name,
            w!("screencap window picker"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            900,
            560,
            None,
            None,
            Some(instance),
            Some(&mut state as *mut GuiState as *const c_void),
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => return 1,
        };

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        msg.wParam.0 as i32
    }
}
