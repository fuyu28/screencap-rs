//! screencap GUI entry point.
#![windows_subsystem = "windows"]

mod gui;

fn main() {
    // DPI awareness is set inside gui::run_gui before any window is created.
    std::process::exit(gui::run_gui());
}
