//! Port of src/gui.cpp: win32 window-picker GUI. Lists capturable windows in
//! a ListView, lets the user pick method/output path, and shells out to
//! screencap-cli.exe (next to this exe) to do the actual capture.

/// Runs the GUI message loop; returns the process exit code.
pub fn run_gui() -> i32 {
    todo!("port RunGui (src/gui.cpp) + SetProcessDpiAwarenessContext from gui_main.cpp")
}
