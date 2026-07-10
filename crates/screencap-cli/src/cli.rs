use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use screencap_core::types::*;
use screencap_core::util::validate_output_path;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_F1, VK_SNAPSHOT, VK_SPACE,
};

#[derive(Clone, Debug)]
pub struct ParsedArgs {
    pub command: CommandType,
    pub common: CommonOptions,
    pub cap: CapOptions,
    pub raw_args: String,
}

#[derive(Parser, Debug)]
#[command(
    name = "screencap-cli",
    about = "Windows screenshot comparison CLI",
    disable_help_subcommand = true
)]
struct CliArgs {
    #[command(flatten)]
    common: CommonCliOptions,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Debug)]
struct CommonCliOptions {
    #[arg(long, global = true, default_value = "./logs")]
    log_dir: String,

    #[arg(long, global = true, value_enum, default_value = "info")]
    log_level: LogLevelArg,

    #[arg(long, global = true)]
    json: bool,

    #[arg(long, global = true, default_value_t = 700, value_parser = clap::value_parser!(i32).range(0..))]
    timeout_ms: i32,

    #[arg(long, global = true, default_value_t = 0, value_parser = clap::value_parser!(i32).range(0..))]
    retry: i32,

    #[arg(long, global = true)]
    overwrite: bool,

    #[arg(long, global = true, value_enum, default_value = "per-monitor-v2")]
    dpi_mode: DpiModeArg,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Cap(Box<CapCli>),
    List(ListCli),
}

#[derive(Args, Debug)]
struct ListCli {
    #[command(subcommand)]
    command: ListCommand,
}

#[derive(Subcommand, Debug)]
enum ListCommand {
    Windows,
    Monitors,
}

#[derive(Args, Debug)]
struct CapCli {
    #[arg(long)]
    method: String,

    #[arg(long, value_enum, default_value = "window")]
    target: TargetArg,

    #[arg(long = "out")]
    out_path: String,

    #[arg(long)]
    stdout: bool,

    #[arg(long)]
    hwnd: Option<u64>,

    #[arg(long)]
    pid: Option<i32>,

    #[arg(long)]
    foreground: bool,

    #[arg(long)]
    title: Option<String>,

    #[arg(long = "class")]
    class_name: Option<String>,

    #[arg(long)]
    monitor: Option<String>,

    #[arg(long)]
    virtual_screen: bool,

    #[arg(long, value_enum, default_value = "none")]
    crop: CropArg,

    #[arg(long, num_args = 4, value_names = ["X", "Y", "W", "H"])]
    crop_rect: Option<Vec<i32>>,

    #[arg(long, num_args = 4, value_names = ["L", "T", "R", "B"])]
    pad: Option<Vec<i32>>,

    #[arg(long, default_value = "png")]
    format: String,

    #[arg(long)]
    force_alpha: Option<i32>,

    #[arg(long)]
    hotkey: Option<String>,

    #[arg(long)]
    hotkey_foreground: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LogLevelArg {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DpiModeArg {
    Auto,
    PerMonitorV2,
    System,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TargetArg {
    Window,
    Screen,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CropArg {
    None,
    Window,
    Client,
    DwmFrame,
    Manual,
}

impl From<CommonCliOptions> for CommonOptions {
    fn from(value: CommonCliOptions) -> Self {
        Self {
            log_dir: value.log_dir,
            log_level: value.log_level.into(),
            json: value.json,
            timeout_ms: value.timeout_ms,
            retry: value.retry,
            overwrite: value.overwrite,
            dpi_mode: value.dpi_mode.into(),
        }
    }
}

impl From<LogLevelArg> for LogLevel {
    fn from(value: LogLevelArg) -> Self {
        match value {
            LogLevelArg::Trace => LogLevel::Trace,
            LogLevelArg::Debug => LogLevel::Debug,
            LogLevelArg::Info => LogLevel::Info,
            LogLevelArg::Warn => LogLevel::Warn,
            LogLevelArg::Error => LogLevel::Error,
        }
    }
}

impl From<DpiModeArg> for DpiMode {
    fn from(value: DpiModeArg) -> Self {
        match value {
            DpiModeArg::Auto => DpiMode::Auto,
            DpiModeArg::PerMonitorV2 => DpiMode::PerMonitorV2,
            DpiModeArg::System => DpiMode::System,
        }
    }
}

impl From<TargetArg> for TargetType {
    fn from(value: TargetArg) -> Self {
        match value {
            TargetArg::Window => TargetType::Window,
            TargetArg::Screen => TargetType::Screen,
        }
    }
}

impl From<CropArg> for CropMode {
    fn from(value: CropArg) -> Self {
        match value {
            CropArg::None => CropMode::None,
            CropArg::Window => CropMode::Window,
            CropArg::Client => CropMode::Client,
            CropArg::DwmFrame => CropMode::DwmFrame,
            CropArg::Manual => CropMode::Manual,
        }
    }
}

fn parse_function_key(token: &str) -> Option<u32> {
    let n = token.strip_prefix('f')?.parse::<i32>().ok()?;
    (1..=24)
        .contains(&n)
        .then_some(VK_F1.0 as u32 + (n - 1) as u32)
}

fn parse_hotkey(spec: &str) -> Option<(u32, u32)> {
    let mut mods: u32 = MOD_NOREPEAT.0;
    let mut vk: u32 = 0;
    let mut has_modifier = false;

    for token in spec
        .trim_end_matches('+')
        .split('+')
        .map(str::to_ascii_lowercase)
    {
        if token.is_empty() {
            return None;
        }

        match token.as_str() {
            "ctrl" | "control" => {
                mods |= MOD_CONTROL.0;
                has_modifier = true;
                continue;
            }
            "alt" => {
                mods |= MOD_ALT.0;
                has_modifier = true;
                continue;
            }
            "shift" => {
                mods |= MOD_SHIFT.0;
                has_modifier = true;
                continue;
            }
            "win" | "windows" => {
                mods |= MOD_WIN.0;
                has_modifier = true;
                continue;
            }
            _ => {}
        }

        if vk != 0 {
            return None;
        }

        if token.len() == 1 {
            let c = token.as_bytes()[0];
            if c.is_ascii_lowercase() {
                vk = (b'A' as u32) + (c - b'a') as u32;
                continue;
            }
            if c.is_ascii_digit() {
                vk = c as u32;
                continue;
            }
            return None;
        }

        if let Some(v) = parse_function_key(&token) {
            vk = v;
            continue;
        }

        match token.as_str() {
            "printscreen" | "prtsc" | "snapshot" => vk = VK_SNAPSHOT.0 as u32,
            "space" => vk = VK_SPACE.0 as u32,
            _ => return None,
        }
    }

    (has_modifier && vk != 0).then_some((mods, vk))
}

fn validation_error(message: impl Into<String>) -> clap::Error {
    clap::Error::raw(ErrorKind::ValueValidation, message.into())
}

impl CapCli {
    fn into_options(self) -> Result<CapOptions, clap::Error> {
        if self.stdout {
            return Err(validation_error(
                "--stdout is not supported in this version",
            ));
        }
        if self.format != "png" {
            return Err(validation_error("only --format png is supported"));
        }
        if let Err(reason) = validate_output_path(&self.out_path) {
            return Err(validation_error(reason));
        }

        let crop_rect = self.crop_rect.map(|values| CropRect {
            x: values[0],
            y: values[1],
            w: values[2],
            h: values[3],
        });
        let crop_mode = self.crop.into();
        if crop_mode == CropMode::Manual && crop_rect.is_none() {
            return Err(validation_error("manual crop needs --crop-rect"));
        }

        let pad = self
            .pad
            .map(|values| Pad {
                l: values[0],
                t: values[1],
                r: values[2],
                b: values[3],
            })
            .unwrap_or_default();

        let mut window_query = TargetWindowQuery {
            hwnd: self.hwnd,
            pid: self.pid,
            foreground: self.foreground,
            title: self.title,
            class_name: self.class_name,
        };
        let screen_query = TargetScreenQuery {
            monitor: self.monitor,
            virtual_screen: self.virtual_screen,
        };
        let target = self.target.into();

        if target == TargetType::Window {
            let has_window_target = window_query.hwnd.is_some()
                || window_query.pid.is_some()
                || window_query.foreground
                || window_query.title.is_some()
                || window_query.class_name.is_some();
            if !has_window_target {
                return Err(validation_error(
                    "window target needs one of --hwnd/--pid/--foreground/--title/--class",
                ));
            }
        } else if screen_query.monitor.is_none() && !screen_query.virtual_screen {
            return Err(validation_error(
                "screen target needs --monitor or --virtual-screen",
            ));
        }

        let force_alpha_255 = match self.force_alpha {
            Some(255) => true,
            Some(_) => return Err(validation_error("--force-alpha only supports 255")),
            None => false,
        };

        let (hotkey_enabled, hotkey_spec, hotkey_modifiers, hotkey_vk) = match self.hotkey {
            Some(spec) => {
                let (mods, vk) = parse_hotkey(&spec).ok_or_else(|| {
                    validation_error("invalid --hotkey (ex: ctrl+shift+s, alt+f9)")
                })?;
                (true, spec, mods, vk)
            }
            None if self.hotkey_foreground => {
                return Err(validation_error("--hotkey-foreground needs --hotkey"));
            }
            None => (false, String::new(), 0, 0),
        };

        if self.hotkey_foreground {
            window_query.foreground = true;
        }

        Ok(CapOptions {
            method: self.method,
            target,
            out_path: self.out_path,
            hotkey_enabled,
            hotkey_spec,
            hotkey_modifiers,
            hotkey_vk,
            window_query,
            screen_query,
            crop_mode,
            crop_rect,
            pad,
            force_alpha_255,
        })
    }
}

pub fn parse_args(argv: &[String]) -> Result<ParsedArgs, clap::Error> {
    if argv.len() <= 1 || argv.get(1).is_some_and(|arg| arg == "help") {
        let mut command = CliArgs::command();
        let help = command.render_long_help().to_string();
        return Err(command.error(ErrorKind::DisplayHelp, help));
    }

    let cli = CliArgs::try_parse_from(argv)?;
    let raw_args = argv.join(" ");
    let common = cli.common.into();

    let (command, cap) = match cli.command {
        Commands::Cap(cap) => (CommandType::Cap, cap.into_options()?),
        Commands::List(list) => match list.command {
            ListCommand::Windows => (CommandType::ListWindows, CapOptions::default()),
            ListCommand::Monitors => (CommandType::ListMonitors, CapOptions::default()),
        },
    };

    Ok(ParsedArgs {
        command,
        common,
        cap,
        raw_args,
    })
}

pub fn is_help_error(err: &clap::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    )
}

pub fn target_type_name(t: TargetType) -> &'static str {
    match t {
        TargetType::Window => "window",
        TargetType::Screen => "screen",
    }
}

pub fn crop_mode_name(m: CropMode) -> &'static str {
    match m {
        CropMode::None => "none",
        CropMode::Window => "window",
        CropMode::Client => "client",
        CropMode::DwmFrame => "dwm-frame",
        CropMode::Manual => "manual",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_global_json_after_list_subcommand() {
        let parsed = parse_args(&args(&["screencap-cli", "list", "windows", "--json"]))
            .expect("list windows should parse");

        assert_eq!(parsed.command, CommandType::ListWindows);
        assert!(parsed.common.json);
    }

    #[test]
    fn parses_screen_capture_options() {
        let parsed = parse_args(&args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-monitor",
            "--target",
            "screen",
            "--monitor",
            "primary",
            "--out",
            "a.png",
        ]))
        .expect("screen capture should parse");

        assert_eq!(parsed.command, CommandType::Cap);
        assert_eq!(parsed.cap.method, "wgc-monitor");
        assert_eq!(parsed.cap.target, TargetType::Screen);
        assert_eq!(parsed.cap.screen_query.monitor.as_deref(), Some("primary"));
        assert_eq!(parsed.cap.out_path, "a.png");
    }

    #[test]
    fn validates_window_target_query() {
        let err = parse_args(&args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-window",
            "--out",
            "a.png",
        ]))
        .expect_err("window capture without target query should fail");

        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn no_args_returns_help() {
        let err = parse_args(&args(&["screencap-cli"])).expect_err("no args should show help");
        assert!(is_help_error(&err));
    }

    /// Minimal valid `cap` argv targeting a foreground window; callers append
    /// extra flags to exercise specific validation branches.
    fn base_window_cap() -> Vec<String> {
        args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-window",
            "--foreground",
            "--out",
            "a.png",
        ])
    }

    #[test]
    fn maps_window_query_and_defaults() {
        let parsed = parse_args(&base_window_cap()).expect("should parse");
        assert_eq!(parsed.command, CommandType::Cap);
        assert_eq!(parsed.cap.target, TargetType::Window);
        assert!(parsed.cap.window_query.foreground);
        assert_eq!(parsed.cap.crop_mode, CropMode::None);
        assert!(parsed.cap.crop_rect.is_none());
        assert_eq!(parsed.cap.pad, Pad::default());
        assert!(!parsed.cap.force_alpha_255);
        assert!(!parsed.cap.hotkey_enabled);
    }

    #[test]
    fn rejects_non_png_format() {
        let mut argv = base_window_cap();
        argv.extend(args(&["--format", "jpg"]));
        let err = parse_args(&argv).expect_err("non-png format should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn rejects_stdout_flag() {
        let mut argv = base_window_cap();
        argv.push("--stdout".to_string());
        let err = parse_args(&argv).expect_err("--stdout should be unsupported");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn manual_crop_requires_crop_rect() {
        let mut argv = base_window_cap();
        argv.extend(args(&["--crop", "manual"]));
        let err = parse_args(&argv).expect_err("manual crop without rect should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn manual_crop_with_rect_maps_values() {
        let mut argv = base_window_cap();
        argv.extend(args(&[
            "--crop",
            "manual",
            "--crop-rect",
            "10",
            "20",
            "30",
            "40",
        ]));
        let parsed = parse_args(&argv).expect("manual crop with rect should parse");
        assert_eq!(parsed.cap.crop_mode, CropMode::Manual);
        assert_eq!(
            parsed.cap.crop_rect,
            Some(CropRect {
                x: 10,
                y: 20,
                w: 30,
                h: 40,
            })
        );
    }

    #[test]
    fn pad_values_map_in_ltrb_order() {
        let mut argv = base_window_cap();
        argv.extend(args(&["--pad", "1", "2", "3", "4"]));
        let parsed = parse_args(&argv).expect("pad should parse");
        assert_eq!(
            parsed.cap.pad,
            Pad {
                l: 1,
                t: 2,
                r: 3,
                b: 4,
            }
        );
    }

    #[test]
    fn force_alpha_only_accepts_255() {
        let mut ok = base_window_cap();
        ok.extend(args(&["--force-alpha", "255"]));
        assert!(
            parse_args(&ok)
                .expect("255 should parse")
                .cap
                .force_alpha_255
        );

        let mut bad = base_window_cap();
        bad.extend(args(&["--force-alpha", "128"]));
        let err = parse_args(&bad).expect_err("non-255 force-alpha should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn hotkey_spec_parses_modifiers_and_key() {
        let mut argv = base_window_cap();
        argv.extend(args(&["--hotkey", "ctrl+shift+s"]));
        let parsed = parse_args(&argv).expect("valid hotkey should parse");
        assert!(parsed.cap.hotkey_enabled);
        assert_eq!(parsed.cap.hotkey_spec, "ctrl+shift+s");
        // 'S' virtual key.
        assert_eq!(parsed.cap.hotkey_vk, b'S' as u32);
        // Modifiers include control and shift bits (NOREPEAT is always set).
        let expected = MOD_NOREPEAT.0 | MOD_CONTROL.0 | MOD_SHIFT.0;
        assert_eq!(parsed.cap.hotkey_modifiers, expected);
    }

    #[test]
    fn hotkey_without_modifier_is_invalid() {
        let mut argv = base_window_cap();
        argv.extend(args(&["--hotkey", "s"]));
        let err = parse_args(&argv).expect_err("hotkey without modifier should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn hotkey_foreground_requires_hotkey() {
        // --hotkey-foreground alone (no --hotkey) is rejected; use an explicit
        // window target so we exercise the hotkey branch, not the target check.
        let argv = args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-window",
            "--hwnd",
            "1234",
            "--out",
            "a.png",
            "--hotkey-foreground",
        ]);
        let err = parse_args(&argv).expect_err("hotkey-foreground without hotkey should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn hotkey_foreground_forces_window_query() {
        let mut argv = args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-window",
            "--hwnd",
            "1234",
            "--out",
            "a.png",
        ]);
        argv.extend(args(&["--hotkey", "alt+f9", "--hotkey-foreground"]));
        let parsed = parse_args(&argv).expect("hotkey-foreground with hotkey should parse");
        assert!(parsed.cap.hotkey_enabled);
        assert!(parsed.cap.window_query.foreground);
    }

    #[test]
    fn screen_target_requires_monitor_or_virtual() {
        let argv = args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-monitor",
            "--target",
            "screen",
            "--out",
            "a.png",
        ]);
        let err = parse_args(&argv).expect_err("screen target needs monitor/virtual");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn virtual_screen_target_parses() {
        let argv = args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-monitor",
            "--target",
            "screen",
            "--virtual-screen",
            "--out",
            "a.png",
        ]);
        let parsed = parse_args(&argv).expect("virtual-screen should parse");
        assert_eq!(parsed.cap.target, TargetType::Screen);
        assert!(parsed.cap.screen_query.virtual_screen);
    }

    /// Minimal valid `cap` argv with a caller-chosen `--out` value.
    fn window_cap_with_out(out: &str) -> Vec<String> {
        args(&[
            "screencap-cli",
            "cap",
            "--method",
            "wgc-window",
            "--foreground",
            "--out",
            out,
        ])
    }

    #[test]
    fn rejects_out_path_with_invalid_character() {
        let err = parse_args(&window_cap_with_out("shot?.png"))
            .expect_err("output path with an invalid character should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn accepts_normal_out_path() {
        let parsed =
            parse_args(&window_cap_with_out("C:/tmp/a.png")).expect("normal path should parse");
        assert_eq!(parsed.cap.out_path, "C:/tmp/a.png");
    }

    // The no-file-name rejection relies on Windows `Path::file_name` semantics.
    #[cfg(windows)]
    #[test]
    fn rejects_out_path_with_no_file_name() {
        let err =
            parse_args(&window_cap_with_out(r"C:\")).expect_err("root output path should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn list_defaults_do_not_require_cap_flags() {
        let parsed = parse_args(&args(&["screencap-cli", "list", "monitors"]))
            .expect("list monitors should parse");
        assert_eq!(parsed.command, CommandType::ListMonitors);
    }
}
