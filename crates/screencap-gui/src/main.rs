//! screencap (GUI window picker) entry point. Port of src/gui_main.cpp.
#![windows_subsystem = "windows"]

mod gui;

fn main() {
    // DPI awareness is set inside gui::run_gui before any window is created.
    std::process::exit(gui::run_gui());
}
