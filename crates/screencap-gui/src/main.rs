//! screencap GUI entry point.
#![windows_subsystem = "windows"]

mod gui;

fn main() {
    std::process::exit(gui::run_gui());
}
