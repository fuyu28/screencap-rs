//! screencap-cli entry point. Port of src/main.cpp.

mod cli;
mod run;

fn main() {
    std::process::exit(run::run());
}
