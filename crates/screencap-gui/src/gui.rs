//! Win32 capture GUI. Lets the user pick a window or monitor (ListView),
//! plus format/output path, and shells out to screencap-cli.exe (next to
//! this exe) for the actual capture.

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
    BST_CHECKED, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
    LIST_VIEW_ITEM_STATE_FLAGS, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_PARAM, LVIF_TEXT,
    LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETEALLITEMS, LVM_DELETECOLUMN, LVM_GETITEMW,
    LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_EX_GRIDLINES, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL, NM_DBLCLK, NMHDR, WC_LISTVIEWW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BS_AUTOCHECKBOX, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
    CBS_DROPDOWNLIST, CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DispatchMessageW, ES_AUTOHSCROLL, GA_ROOT, GWLP_USERDATA, GetAncestor, GetClientRect,
    GetMessageW, GetWindowLongPtrW, HMENU, IDC_ARROW, LoadCursorW, MB_ICONERROR,
    MB_ICONINFORMATION, MSG, MessageBoxW, MoveWindow, PostMessageW, PostQuitMessage,
    RegisterClassW, SW_SHOW, SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_NCCREATE, WM_NOTIFY, WM_SIZE, WNDCLASSW, WS_CHILD, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use screencap_core::encode_png::{normalize_path_separators, output_parent_dir, real_output_path};
use screencap_core::monitor_enum::enumerate_monitors;
use screencap_core::types::{ImageFormat, MonitorInfo, Rect, WindowInfo};
use screencap_core::util::{
    build_timestamp_for_filename, utf8_from_wide, validate_output_path, wide_from_utf8,
};
use screencap_core::window_enum::{enumerate_windows, get_window_text_utf8};

const ID_LIST: u16 = 1001;
const ID_REFRESH: u16 = 1002;
const ID_TARGET: u16 = 1003;
const ID_OUT: u16 = 1004;
const ID_BROWSE: u16 = 1005;
const ID_CAPTURE: u16 = 1006;
const ID_STATUS: u16 = 1007;
const ID_FORMAT: u16 = 1008;
const ID_CURSOR: u16 = 1009;

/// GUI target-type combo entries. Index maps 1:1 to the [`GuiTarget`] variants.
const TARGET_LABELS: [&str; 2] = ["Window", "Monitor"];

const WINDOW_COLUMNS: [(&str, i32); 4] =
    [("Title", 360), ("Class", 170), ("PID", 80), ("Rect", 180)];
const MONITOR_COLUMNS: [(&str, i32); 5] = [
    ("Index", 60),
    ("Name", 200),
    ("Size", 120),
    ("Primary", 80),
    ("Rect", 200),
];

/// Posted from the capture worker thread to the GUI thread once
/// screencap-cli.exe has finished (or failed to start). `WPARAM` is 1 for
/// success, 0 for failure; on failure `LPARAM` carries a pointer to a
/// heap-allocated `String` (boxed via `Box::into_raw`) with the exact error
/// text, which the handler reclaims with `Box::from_raw`.
const WM_APP_CAPTURE_DONE: u32 = WM_APP + 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GuiTarget {
    #[default]
    Window,
    Monitor,
}

/// Resolved capture target carried to the worker thread (both variants are `Send`).
enum CaptureTarget {
    Window(usize),
    Monitor(i32),
}

impl CaptureTarget {
    /// Returns the CLI `--method` allowlist string for this target.
    fn method(&self) -> &'static str {
        match self {
            CaptureTarget::Window(_) => "wgc-window",
            CaptureTarget::Monitor(_) => "wgc-monitor",
        }
    }
}

/// Per-window state. A pointer to this struct is stored in GWLP_USERDATA so the
/// window procedure can recover its context.
#[derive(Default)]
struct GuiState {
    hwnd: HWND,
    list: HWND,
    refresh: HWND,
    target: HWND,
    format: HWND,
    cursor: HWND,
    out: HWND,
    browse: HWND,
    capture: HWND,
    status: HWND,
    windows: Vec<WindowInfo>,
    monitors: Vec<MonitorInfo>,
    /// Target type currently shown in the ListView (may lag the combo briefly
    /// during `CBN_SELCHANGE` handling).
    list_target: GuiTarget,
    /// Last chosen monitor `index`, kept across Window/Monitor target switches
    /// so returning to Monitor restores the same row (combo used to keep this).
    preferred_monitor: Option<i32>,
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

/// NUL-padded wide buffer of fixed `len` for Win32 dialog structures (e.g. `OPENFILENAMEW`).
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

/// Shows an informational message box owned by the GUI window.
fn info_box(hwnd: HWND, text: &str) {
    unsafe {
        MessageBoxW(
            Some(hwnd),
            &HSTRING::from(text),
            w!("screencap"),
            MB_ICONINFORMATION,
        );
    }
}

/// Default output path under the current directory using the selected format extension.
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

/// Lays out child controls to fill the main window client area (`WM_SIZE`).
fn resize_controls(state: &GuiState) {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(state.hwnd, &mut rc);
    }
    let pad = 10;
    let button_h = 28;
    let out_h = 24;
    let status_h = 22;
    let target_w = 110;
    let format_w = 90;
    let cursor_w = 130;
    let browse_w = 80;
    let capture_w = 92;
    let refresh_w = 80;
    let width = rc.right - rc.left;
    let height = rc.bottom - rc.top;

    unsafe {
        let _ = MoveWindow(state.refresh, pad, pad, refresh_w, button_h, true);
        let _ = MoveWindow(
            state.target,
            pad + refresh_w + pad,
            pad,
            target_w,
            180,
            true,
        );
        let _ = MoveWindow(
            state.format,
            pad + refresh_w + pad + target_w + pad,
            pad,
            format_w,
            180,
            true,
        );
        let _ = MoveWindow(
            state.cursor,
            pad + refresh_w + pad + target_w + pad + format_w + pad,
            pad,
            cursor_w,
            button_h,
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

        let pick_y = out_y + out_h + pad;
        let pick_h = height - pick_y - status_h - pad * 2;
        let _ = MoveWindow(
            state.list,
            pad,
            pick_y,
            width - pad * 2,
            pick_h.max(80),
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

/// Removes every ListView column (needed before switching Window/Monitor schemas).
fn clear_list_columns(list: HWND) {
    loop {
        let ok = unsafe { SendMessageW(list, LVM_DELETECOLUMN, Some(WPARAM(0)), Some(LPARAM(0))) };
        if ok.0 == 0 {
            break;
        }
    }
}

/// Replaces ListView columns with `columns` (clears existing columns first).
fn set_list_columns(list: HWND, columns: &[(&str, i32)]) {
    clear_list_columns(list);
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

/// Sets a ListView cell via `LVM_SETITEMTEXTW`.
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

/// Selects and focuses a ListView row.
fn select_list_item(list: HWND, item: i32) {
    let flags = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
    let lv = LVITEMW {
        state: flags,
        stateMask: flags,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            Some(WPARAM(item as usize)),
            Some(LPARAM(&lv as *const LVITEMW as isize)),
        );
    }
}

/// Returns true for visible, unminimized, uncloaked top-level windows with a title.
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

/// Formats a rect for a ListView cell.
fn format_rect(rect: &Rect) -> String {
    format!(
        "{},{} {}x{}",
        rect.left,
        rect.top,
        rect.width(),
        rect.height()
    )
}

/// Clears ListView rows and inserts one row per cached window.
fn populate_window_rows(state: &GuiState) {
    unsafe {
        SendMessageW(
            state.list,
            LVM_DELETEALLITEMS,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
    }

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
        set_item_text(state.list, i as i32, 3, &format_rect(&w.rect));
    }
}

/// Clears ListView rows and inserts one row per cached monitor.
fn populate_monitor_rows(state: &GuiState, select: Option<usize>) {
    unsafe {
        SendMessageW(
            state.list,
            LVM_DELETEALLITEMS,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
    }

    for (i, m) in state.monitors.iter().enumerate() {
        let mut index_w = to_wide(&m.index.to_string());
        let item = LVITEMW {
            mask: LVIF_TEXT | LVIF_PARAM,
            iItem: i as i32,
            pszText: PWSTR(index_w.as_mut_ptr()),
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
        set_item_text(state.list, i as i32, 1, &m.name);
        set_item_text(
            state.list,
            i as i32,
            2,
            &format!("{}x{}", m.desktop.width(), m.desktop.height()),
        );
        set_item_text(state.list, i as i32, 3, if m.primary { "yes" } else { "" });
        set_item_text(state.list, i as i32, 4, &format_rect(&m.desktop));
    }

    if let Some(sel) = select {
        select_list_item(state.list, sel as i32);
    }
}

/// Reconfigures ListView columns/rows for the current target type.
fn populate_list(state: &mut GuiState) {
    let target = selected_target(state);
    match target {
        GuiTarget::Window => {
            set_list_columns(state.list, &WINDOW_COLUMNS);
            populate_window_rows(state);
        }
        GuiTarget::Monitor => {
            set_list_columns(state.list, &MONITOR_COLUMNS);
            let sel = restore_monitor_selection(&state.monitors, state.preferred_monitor);
            populate_monitor_rows(state, sel);
            state.preferred_monitor = sel.and_then(|i| state.monitors.get(i).map(|m| m.index));
        }
    }
    state.list_target = target;
}

/// Enumerates pickable windows into [`GuiState::windows`].
fn reload_windows(state: &mut GuiState) {
    let mut pickable: Vec<WindowInfo> = enumerate_windows()
        .into_iter()
        .filter(is_pickable)
        .collect();
    pickable.sort_by(|a, b| a.title.cmp(&b.title));
    state.windows = pickable;
}

/// Picks the ListView row to select after a monitor refresh.
/// Prefers the previously selected `monitor.index`; if that monitor is gone,
/// falls back to primary, then row 0. Returns `None` when the list is empty.
fn restore_monitor_selection(monitors: &[MonitorInfo], prev_index: Option<i32>) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }
    Some(
        prev_index
            .and_then(|idx| monitors.iter().position(|m| m.index == idx))
            .or_else(|| monitors.iter().position(|m| m.primary))
            .unwrap_or(0),
    )
}

/// Enumerates monitors into [`GuiState::monitors`].
fn reload_monitors(state: &mut GuiState) {
    let mut monitors = enumerate_monitors();
    monitors.sort_by_key(|m| m.index);
    state.monitors = monitors;
}

/// Re-enumerates the current target type and rebuilds the ListView.
fn refresh_current_target(state: &mut GuiState) {
    match selected_target(state) {
        GuiTarget::Window => {
            reload_windows(state);
            set_list_columns(state.list, &WINDOW_COLUMNS);
            populate_window_rows(state);
            set_status(state, &format!("Windows: {}", state.windows.len()));
        }
        GuiTarget::Monitor => {
            // Capture before list replace so Refresh keeps the same monitor
            // instead of silently snapping back to primary.
            let prev_index = selected_monitor_index(state).or(state.preferred_monitor);
            reload_monitors(state);
            set_list_columns(state.list, &MONITOR_COLUMNS);
            let sel = restore_monitor_selection(&state.monitors, prev_index);
            populate_monitor_rows(state, sel);
            state.preferred_monitor = sel.and_then(|i| state.monitors.get(i).map(|m| m.index));
            set_status(state, &format!("Monitors: {}", state.monitors.len()));
        }
    }
}

/// Remembers the ListView monitor selection before swapping to Window columns.
fn remember_monitor_selection(state: &mut GuiState) {
    if state.list_target == GuiTarget::Monitor
        && let Some(index) = selected_monitor_index(state)
    {
        state.preferred_monitor = Some(index);
    }
}

/// Updates the status line from the cached count for the current target type.
fn update_target_status(state: &GuiState) {
    match selected_target(state) {
        GuiTarget::Window => set_status(state, &format!("Windows: {}", state.windows.len())),
        GuiTarget::Monitor => set_status(state, &format!("Monitors: {}", state.monitors.len())),
    }
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

/// Opens the save-file dialog and writes the chosen path into the output edit control.
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

/// Returns the target-type combobox selection.
fn selected_target(state: &GuiState) -> GuiTarget {
    combo_selection(state.target, &[GuiTarget::Window, GuiTarget::Monitor])
}

/// Returns the output-format combobox selection.
fn selected_format(state: &GuiState) -> ImageFormat {
    combo_selection(state.format, &ImageFormat::ALL)
}

/// Whether the "Include cursor" checkbox is currently checked.
fn cursor_included(state: &GuiState) -> bool {
    let checked = unsafe { SendMessageW(state.cursor, BM_GETCHECK, None, None) };
    checked.0 == BST_CHECKED.0 as isize
}

/// Rewrites the output-path extension to match the selected format so the
/// default timestamp filename tracks the format combobox. Called after Browse
/// because the save dialog may leave a mismatched extension.
fn sync_output_extension(state: &GuiState) {
    let current = get_window_text_utf8(state.out);
    if current.is_empty() {
        return;
    }
    let mut path = PathBuf::from(&current);
    path.set_extension(selected_format(state).extension());
    set_window_text(state.out, &path.to_string_lossy());
}

/// Maps the ListView selection to an index via `LVIF_PARAM`.
fn selected_list_index(state: &GuiState) -> Option<usize> {
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

/// Returns the selected monitor's `index` field, if any.
fn selected_monitor_index(state: &GuiState) -> Option<i32> {
    let idx = selected_list_index(state)?;
    state.monitors.get(idx).map(|m| m.index)
}

/// Resolves `screencap-cli.exe` next to the running GUI executable.
fn cli_exe_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent()
        .map(|p| p.join("screencap-cli.exe"))
        .unwrap_or_else(|| PathBuf::from("screencap-cli.exe"))
}

/// Pulls `error.message` out of a `cap --json` failure payload when present.
fn extract_cli_error_message(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    value
        .get("error")?
        .get("message")?
        .as_str()
        .map(str::to_string)
}

/// Shell out to screencap-cli.exe with `CREATE_NO_WINDOW` so no console flashes
/// up. `target` selects the `--target window` vs `--target screen` argv.
fn run_capture_process(
    target: CaptureTarget,
    out_path: &str,
    format: ImageFormat,
    include_cursor: bool,
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
        .arg(target.method())
        .arg("--out")
        .arg(out_path)
        .arg("--overwrite")
        .arg("--json")
        .arg("--no-log")
        .arg("--timeout-ms")
        .arg("2000")
        .arg("--force-alpha")
        .arg("255");

    match target {
        CaptureTarget::Window(hwnd) => {
            command
                .arg("--target")
                .arg("window")
                .arg("--hwnd")
                .arg(hwnd.to_string());
        }
        CaptureTarget::Monitor(index) => {
            command
                .arg("--target")
                .arg("screen")
                .arg("--monitor")
                .arg(index.to_string());
        }
    }

    // Do not pass --format when it matches the CLI default; keeps argv minimal.
    if format != ImageFormat::default() {
        command.arg("--format").arg(format.as_str());
    }

    // Do not pass --cursor unless opted in; CLI excludes the cursor by default.
    if include_cursor {
        command.arg("--cursor");
    }

    let output = command.creation_flags(CREATE_NO_WINDOW).output();

    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let message = extract_cli_error_message(&stdout).unwrap_or_else(|| {
                format!(
                    "Capture failed. Exit code: {}",
                    output.status.code().unwrap_or(1)
                )
            });
            Err(message)
        }
        Err(e) => Err(format!("Failed to start screencap-cli.exe: {e}")),
    }
}

/// Kicks off a capture on a worker thread so the message loop stays
/// responsive (screencap-cli.exe can take up to ~10s with WGC retries).
/// Completion is reported back via `WM_APP_CAPTURE_DONE`; see
/// [`wnd_proc`]'s handler for that message.
fn capture_selected(state: &mut GuiState) {
    if state.capturing {
        return;
    }

    let target = match selected_target(state) {
        GuiTarget::Window => {
            let idx = match selected_list_index(state) {
                Some(idx) if idx < state.windows.len() => idx,
                _ => {
                    info_box(state.hwnd, "Select a window first.");
                    return;
                }
            };
            CaptureTarget::Window(state.windows[idx].hwnd as usize)
        }
        GuiTarget::Monitor => {
            let idx = match selected_list_index(state) {
                Some(idx) if idx < state.monitors.len() => idx,
                _ => {
                    info_box(state.hwnd, "Select a monitor first.");
                    return;
                }
            };
            CaptureTarget::Monitor(state.monitors[idx].index)
        }
    };
    if let CaptureTarget::Monitor(index) = &target {
        state.preferred_monitor = Some(*index);
    }

    let out_path = get_window_text_utf8(state.out);
    if out_path.is_empty() {
        info_box(state.hwnd, "Choose an output path first.");
        return;
    }

    // Do not defer invalid paths to the CLI: surface a clear dialog here instead
    // of an opaque exit code. `/` is valid on Windows and passes validate_output_path.
    if let Err(reason) = validate_output_path(&out_path) {
        info_box(state.hwnd, &reason);
        return;
    }

    let normalized_out = normalize_path_separators(&out_path);
    // Do not rely on the CLI alone for a missing parent directory; check here
    // after the same separator normalization the backend uses.
    if let Some(parent) = output_parent_dir(&normalized_out)
        && !std::fs::metadata(parent)
            .map(|m| m.is_dir())
            .unwrap_or(false)
    {
        info_box(
            state.hwnd,
            &format!("output directory does not exist: {parent}"),
        );
        return;
    }

    let format = selected_format(state);
    let include_cursor = cursor_included(state);

    state.capturing = true;
    state.pending_out = out_path.clone();
    unsafe {
        let _ = EnableWindow(state.capture, false);
    }
    set_status(state, "Capturing...");
    unsafe {
        let _ = UpdateWindow(state.hwnd);
    }

    // HWND is not Send; carry raw bits and rebuild on the worker thread.
    let hwnd_raw = state.hwnd.0 as isize;
    std::thread::spawn(move || {
        let result = run_capture_process(target, &out_path, format, include_cursor);
        let (wparam, lparam): (usize, isize) = match result {
            Ok(()) => (1, 0),
            Err(err) => (0, Box::into_raw(Box::new(err)) as isize),
        };
        let hwnd = HWND(hwnd_raw as *mut c_void);
        // Do not block the UI thread on CLI I/O; PostMessageW defers completion to wnd_proc.
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
        let real = real_output_path(&state.pending_out);
        set_status(state, &format!("Saved: {real}"));
        return;
    }

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

/// Creates a child window with the given control ID; returns a null HWND on failure.
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

/// Creates all child controls and performs the initial list refresh.
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

    state.target = create_combo(hwnd, instance, ID_TARGET, &TARGET_LABELS);
    state.format = create_combo(
        hwnd,
        instance,
        ID_FORMAT,
        &ImageFormat::ALL.map(|f| f.as_str()),
    );

    state.cursor = create_child(
        hwnd,
        instance,
        Default::default(),
        w!("BUTTON"),
        w!("Include cursor"),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        ID_CURSOR,
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
    reload_windows(state);
    reload_monitors(state);
    populate_list(state);
    update_target_status(state);
}

/// Main window procedure: creates controls, handles layout, commands, ListView
/// double-click, async capture completion (`WM_APP_CAPTURE_DONE`), and shutdown.
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
                    refresh_current_target(state);
                    return LRESULT(0);
                } else if id == ID_BROWSE {
                    browse_output(state);
                    return LRESULT(0);
                } else if id == ID_CAPTURE {
                    capture_selected(state);
                    return LRESULT(0);
                } else if id == ID_TARGET && code == CBN_SELCHANGE as u16 {
                    remember_monitor_selection(state);
                    populate_list(state);
                    update_target_status(state);
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
            w!("screencap"),
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

#[cfg(test)]
mod tests {
    use super::{extract_cli_error_message, restore_monitor_selection};
    use screencap_core::types::{MonitorInfo, Rect};

    fn monitor(index: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            hmon: index as isize,
            index,
            name: format!(r"\\.\DISPLAY{}", index + 1),
            desktop: Rect {
                left: index * 1920,
                top: 0,
                right: (index + 1) * 1920,
                bottom: 1080,
            },
            primary,
        }
    }

    #[test]
    fn extract_cli_error_message_reads_failure_json() {
        let stdout = r#"{"ok":false,"error":{"message":"output exists (use --overwrite)","where":"SavePngWic"}}"#;
        assert_eq!(
            extract_cli_error_message(stdout).as_deref(),
            Some("output exists (use --overwrite)")
        );
    }

    #[test]
    fn extract_cli_error_message_ignores_non_json() {
        assert!(extract_cli_error_message("not json").is_none());
        assert!(extract_cli_error_message(r#"{"ok":true}"#).is_none());
    }

    #[test]
    fn restore_monitor_selection_keeps_prev_index() {
        let monitors = vec![monitor(0, true), monitor(1, false)];
        assert_eq!(restore_monitor_selection(&monitors, Some(1)), Some(1));
    }

    #[test]
    fn restore_monitor_selection_falls_back_to_primary_when_gone() {
        let monitors = vec![monitor(0, false), monitor(2, true)];
        assert_eq!(restore_monitor_selection(&monitors, Some(1)), Some(1));
    }

    #[test]
    fn restore_monitor_selection_empty_list() {
        assert_eq!(restore_monitor_selection(&[], Some(0)), None);
    }
}
