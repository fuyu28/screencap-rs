//! screencap-cli entry point.

mod cli;
mod run;

fn main() {
    std::process::exit(run::run());
}
