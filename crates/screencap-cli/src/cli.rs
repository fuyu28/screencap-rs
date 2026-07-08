//! Port of src/cli.cpp: hand-rolled arg parser (kept bespoke so behavior and
//! error messages match the C++ CLI exactly).

use screencap_core::types::*;

#[derive(Clone, Debug)]
pub struct ParsedArgs {
    pub command: CommandType,
    pub common: CommonOptions,
    pub cap: CapOptions,
    pub raw_args: Vec<String>,
}

impl Default for ParsedArgs {
    fn default() -> Self {
        Self {
            command: CommandType::Help,
            common: CommonOptions::default(),
            cap: CapOptions::default(),
            raw_args: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParseResult {
    pub ok: bool,
    pub show_help: bool,
    pub args: ParsedArgs,
    pub error: String,
}

/// Full CLI grammar of the C++ ParseArgs, including validation
/// (cap needs --method/--out, window target needs a query, --format png only,
/// manual crop needs --crop-rect, --hotkey-foreground needs --hotkey, hotkey
/// spec parsing like ctrl+shift+s / alt+f9).
pub fn parse_args(_argv: &[String]) -> ParseResult {
    todo!("port ParseArgs (src/cli.cpp)")
}

pub fn dpi_mode_name(_mode: DpiMode) -> &'static str {
    todo!()
}

pub fn target_type_name(_t: TargetType) -> &'static str {
    todo!()
}

pub fn crop_mode_name(_m: CropMode) -> &'static str {
    todo!()
}

pub fn build_help_text() -> String {
    todo!("port BuildHelpText")
}
