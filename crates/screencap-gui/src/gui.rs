//! Win32 window-picker GUI. Lists capturable windows in a ListView, lets the
//! user pick method/output path, and shells out to
//! screencap-cli.exe (next to this exe) to do the actual capture.

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH, UpdateWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, LVCF_TEXT, LVCF_WIDTH,
    LVCOLUMNW, LVIF_PARAM, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS, LVM_GETITEMW, LVM_GETNEXTITEM,
    LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMTEXTW,
    LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_EX_GRIDLINES, LVS_REPORT,
    LVS_SHOWSELALWAYS, LVS_SINGLESEL, NM_DBLCLK, NMHDR, WC_LISTVIEWW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST, CREATESTRUCTW,
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, ES_AUTOHSCROLL, GA_ROOT,
    GWLP_USERDATA, GetAncestor, GetClientRect, GetMessageW, GetWindowLongPtrW, HMENU, IDC_ARROW,
    LoadCursorW, MB_ICONERROR, MB_ICONINFORMATION, MSG, MessageBoxW, MoveWindow, PostMessageW,
    PostQuitMessage, RegisterClassW, SW_SHOW, SendMessageW, SetWindowLongPtrW, SetWindowTextW,
    ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_NCCREATE, WM_NOTIFY, WM_SIZE, WNDCLASSW, WS_CHILD, WS_EX_CLIENTEDGE,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use screencap_core::encode_png::{normalize_path_separators, output_parent_dir, real_output_path};
use screencap_core::types::{ImageFormat, WindowInfo};
use screencap_core::util::{
    build_timestamp_for_filename, utf8_from_wide, validate_output_path, wide_from_utf8,
};
use screencap_core::window_enum::{enumerate_windows, get_window_text_utf8};

const ID_LIST: u16 = 1001;
const ID_REFRESH: u16 = 1002;
const ID_METHOD: u16 = 1003;
const ID_OUT: u16 = 1004;
const ID_BROWSE: u16 = 1005;
const ID_CAPTURE: u16 = 1006;
const ID_STATUS: u16 = 1007;
const ID_FORMAT: u16 = 1008;

/// Posted from the capture worker thread to the GUI thread once
/// screencap-cli.exe has finished (or failed to start). `WPARAM` is 1 for
/// success, 0 for failure; on failure `LPARAM` carries a pointer to a
/// heap-allocated `String` (boxed via `Box::into_raw`) with the exact error
/// text, which the handler reclaims with `Box::from_raw`.
const WM_APP_CAPTURE_DONE: u32 = WM_APP + 1;

const METHODS: [&str; 1] = ["wgc-window"];

/// Per-window state. A pointer to this struct is stored in GWLP_USERDATA so the
/// window procedure can recover its context.
#[derive(Default)]
struct GuiState {
    hwnd: HWND,
    list: HWND,
    refresh: HWND,
    method: HWND,
    format: HWND,
    out: HWND,
    browse: HWND,
    capture: HWND,
    status: HWND,
    windows: Vec<WindowInfo>,
    /// True while a capture worker thread is in flight; further capture
    /// requests are ignored until it completes.
    capturing: bool,
    /// Output path for the in-flight capture, stashed here so the
    /// `WM_APP_CAPTURE_DONE` handler can build the "Saved: ..." status text
    /// without threading it through the posted message.
    pending_out: String,
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

fn set_status(state: &GuiState, text: &str) {
    set_window_text(state.status, text);
}

fn default_output_path() -> String {
    let filename = format!(
        "screenshot_{}.{}",
        build_timestamp_for_filename(),
        ImageFormat::default().extension()
    );
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
    let method_w = 150;
    let format_w = 90;
    let browse_w = 80;
    let capture_w = 92;
    let refresh_w = 80;
    let width = rc.right - rc.left;
    let height = rc.bottom - rc.top;

    unsafe {
        let _ = MoveWindow(state.refresh, pad, pad, refresh_w, button_h, true);
        let _ = MoveWindow(
            state.method,
            pad + refresh_w + pad,
            pad,
            method_w,
            180,
            true,
        );
        let _ = MoveWindow(
            state.format,
            pad + refresh_w + pad + method_w + pad,
            pad,
            format_w,
            180,
            true,
        );
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

/// Builds the Save-dialog filter for the selected format, keeping the
/// All-files entry.
fn build_save_filter(format: ImageFormat) -> Vec<u16> {
    let ext = format.extension();
    let image_entry = format!("{} image (*.{ext})", ext.to_uppercase());
    let image_pattern = format!("*.{ext}");
    let mut buf = Vec::new();
    for part in [
        image_entry.as_str(),
        image_pattern.as_str(),
        "All files (*.*)",
        "*.*",
    ] {
        buf.extend(wide_from_utf8(part));
        buf.push(0);
    }
    buf.push(0);
    buf
}

fn browse_output(state: &mut GuiState) {
    let current = get_window_text_utf8(state.out);
    let mut file_buf = to_wide_fixed(&current, 260);
    let format = selected_format(state);
    let filter = build_save_filter(format);
    let def_ext = to_wide(format.extension());

    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: state.hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrDefExt: PCWSTR(def_ext.as_ptr()),
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
        // The user may have picked the All-files filter or typed a mismatched
        // extension; keep the extension in step with the format combobox.
        sync_output_extension(state);
    }
}

/// Returns the item matching the combobox's current selection, falling back
/// to the first item when nothing is selected (CB_GETCURSEL returns -1, which
/// lands outside the slice after the usize cast).
fn combo_selection<T: Copy>(combo: HWND, items: &[T]) -> T {
    let idx = unsafe { SendMessageW(combo, CB_GETCURSEL, None, None) }.0 as usize;
    items.get(idx).copied().unwrap_or(items[0])
}

fn selected_method(state: &GuiState) -> &'static str {
    combo_selection(state.method, &METHODS)
}

fn selected_format(state: &GuiState) -> ImageFormat {
    combo_selection(state.format, &ImageFormat::ALL)
}

/// Rewrites the output-path extension to match the selected format so the
/// default timestamp filename tracks the format combobox.
fn sync_output_extension(state: &GuiState) {
    let current = get_window_text_utf8(state.out);
    if current.is_empty() {
        return;
    }
    let mut path = PathBuf::from(&current);
    path.set_extension(selected_format(state).extension());
    set_window_text(state.out, &path.to_string_lossy());
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
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent()
        .map(|p| p.join("screencap-cli.exe"))
        .unwrap_or_else(|| PathBuf::from("screencap-cli.exe"))
}

/// Shell out to screencap-cli.exe with `CREATE_NO_WINDOW` so no console flashes
/// up.
fn run_capture_process(
    window: &WindowInfo,
    method: &str,
    out_path: &str,
    format: ImageFormat,
) -> Result<(), String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let cli_path = cli_exe_path();
    if !cli_path.exists() {
        return Err("screencap-cli.exe was not found next to screencap.exe.".to_string());
    }

    let mut command = Command::new(&cli_path);
    command
        .arg("cap")
        .arg("--method")
        .arg(method)
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
        .arg("255");

    // Omit --format for the CLI-default format to keep the command line
    // minimal.
    if format != ImageFormat::default() {
        command.arg("--format").arg(format.as_str());
    }

    let status = command.creation_flags(CREATE_NO_WINDOW).status();

    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "Capture failed. Exit code: {}",
            status.code().unwrap_or(1)
        )),
        Err(e) => Err(format!("Failed to start screencap-cli.exe: {e}")),
    }
}

/// Kicks off a capture on a worker thread so the message loop stays
/// responsive (screencap-cli.exe can take up to ~10s with WGC retries).
/// Completion is reported back via `WM_APP_CAPTURE_DONE`; see
/// [`wnd_proc`]'s handler for that message.
fn capture_selected(state: &mut GuiState) {
    if state.capturing {
        // A capture is already in flight; ignore the request (the Capture
        // button is disabled too, but double-click on the list can still
        // reach here).
        return;
    }

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

    let out_path = get_window_text_utf8(state.out);
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

    // Reject clearly-invalid paths up front with a clear message, rather than
    // letting the capture backend fail opaquely. `/` is a valid separator and
    // passes this check.
    if let Err(reason) = validate_output_path(&out_path) {
        unsafe {
            MessageBoxW(
                Some(state.hwnd),
                &HSTRING::from(reason.as_str()),
                w!("screencap"),
                MB_ICONINFORMATION,
            );
        }
        return;
    }

    // The capture backend refuses to create missing directories, so a bad
    // parent path would otherwise fail with just an exit code. Check it here
    // and report clearly. Normalize first so `/` behaves like the backend.
    let normalized_out = normalize_path_separators(&out_path);
    if let Some(parent) = output_parent_dir(&normalized_out)
        && !std::fs::metadata(parent)
            .map(|m| m.is_dir())
            .unwrap_or(false)
    {
        let msg = format!("output directory does not exist: {parent}");
        unsafe {
            MessageBoxW(
                Some(state.hwnd),
                &HSTRING::from(msg.as_str()),
                w!("screencap"),
                MB_ICONINFORMATION,
            );
        }
        return;
    }

    let window = state.windows[idx].clone();
    let method = selected_method(state);
    let format = selected_format(state);

    state.capturing = true;
    state.pending_out = out_path.clone();
    unsafe {
        let _ = EnableWindow(state.capture, false);
    }
    set_status(state, "Capturing...");
    unsafe {
        let _ = UpdateWindow(state.hwnd);
    }

    // HWND wraps a raw pointer and is not Send; carry the bits across the
    // thread boundary as an isize and rebuild the HWND on the other side.
    let hwnd_raw = state.hwnd.0 as isize;
    std::thread::spawn(move || {
        let result = run_capture_process(&window, method, &out_path, format);
        let (wparam, lparam): (usize, isize) = match result {
            Ok(()) => (1, 0),
            Err(err) => (0, Box::into_raw(Box::new(err)) as isize),
        };
        let hwnd = HWND(hwnd_raw as *mut c_void);
        // PostMessageW is safe to call from a non-UI thread; the wndproc on
        // the GUI thread will pick this up on its next GetMessageW loop.
        let _ = unsafe {
            PostMessageW(
                Some(hwnd),
                WM_APP_CAPTURE_DONE,
                WPARAM(wparam),
                LPARAM(lparam),
            )
        };
    });
}

/// Handles `WM_APP_CAPTURE_DONE`, posted by the worker thread spawned in
/// [`capture_selected`]. Restores the UI to its idle state and shows the
/// same success/failure text the old synchronous path produced.
fn on_capture_done(state: &mut GuiState, wparam: WPARAM, lparam: LPARAM) {
    state.capturing = false;
    unsafe {
        let _ = EnableWindow(state.capture, true);
    }

    if wparam.0 == 1 {
        // Report the real on-disk path: on case-insensitive volumes a request
        // for `test.png` may have landed in an existing `TEST.png`, and the
        // status must name the file that actually exists.
        let real = real_output_path(&state.pending_out);
        set_status(state, &format!("Saved: {real}"));
        return;
    }

    // Reclaim the boxed error string handed off by the worker thread.
    let err = *unsafe { Box::from_raw(lparam.0 as *mut String) };
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

fn create_child(
    parent: HWND,
    instance: HINSTANCE,
    ex_style: WINDOW_EX_STYLE,
    class: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    id: u16,
) -> HWND {
    unsafe {
        CreateWindowExW(
            ex_style,
            class,
            text,
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(control_id(id)),
            Some(instance),
            None,
        )
    }
    .unwrap_or_default()
}

/// Creates a drop-down list child, fills it with `items`, and selects the
/// first entry.
fn create_combo(parent: HWND, instance: HINSTANCE, id: u16, items: &[&str]) -> HWND {
    let combo = create_child(
        parent,
        instance,
        Default::default(),
        w!("COMBOBOX"),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
        id,
    );
    for item in items {
        let wide = to_wide(item);
        unsafe {
            SendMessageW(
                combo,
                CB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(wide.as_ptr() as isize)),
            );
        }
    }
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)));
    }
    combo
}

fn create_controls(state: &mut GuiState, hwnd: HWND) {
    state.hwnd = hwnd;
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map(|h| HINSTANCE(h.0))
        .unwrap_or_default();

    state.refresh = create_child(
        hwnd,
        instance,
        Default::default(),
        w!("BUTTON"),
        w!("Refresh"),
        WS_CHILD | WS_VISIBLE,
        ID_REFRESH,
    );

    state.method = create_combo(hwnd, instance, ID_METHOD, &METHODS);
    state.format = create_combo(
        hwnd,
        instance,
        ID_FORMAT,
        &ImageFormat::ALL.map(|f| f.as_str()),
    );

    state.out = create_child(
        hwnd,
        instance,
        WS_EX_CLIENTEDGE,
        w!("EDIT"),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        ID_OUT,
    );
    set_window_text(state.out, &default_output_path());

    state.browse = create_child(
        hwnd,
        instance,
        Default::default(),
        w!("BUTTON"),
        w!("Browse"),
        WS_CHILD | WS_VISIBLE,
        ID_BROWSE,
    );

    state.capture = create_child(
        hwnd,
        instance,
        Default::default(),
        w!("BUTTON"),
        w!("Capture"),
        WS_CHILD | WS_VISIBLE,
        ID_CAPTURE,
    );

    state.list = create_child(
        hwnd,
        instance,
        WS_EX_CLIENTEDGE,
        WC_LISTVIEWW,
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS),
        ID_LIST,
    );
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

    state.status = create_child(
        hwnd,
        instance,
        Default::default(),
        w!("STATIC"),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE,
        ID_STATUS,
    );

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
                let code = (wparam.0 >> 16) as u16;
                if id == ID_REFRESH {
                    refresh_windows(state);
                    return LRESULT(0);
                } else if id == ID_BROWSE {
                    browse_output(state);
                    return LRESULT(0);
                } else if id == ID_CAPTURE {
                    capture_selected(state);
                    return LRESULT(0);
                } else if id == ID_FORMAT && code == CBN_SELCHANGE as u16 {
                    sync_output_extension(state);
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
        WM_APP_CAPTURE_DONE => {
            if let Some(state) = unsafe { state_ptr.as_mut() } {
                on_capture_done(state, wparam, lparam);
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
