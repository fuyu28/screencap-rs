//! HWND/HMONITOR are stored as `isize` so these structs stay Send/Sync and
//! serialize directly into the JSON output; convert at Win32 call sites.

use serde::{Serialize, Serializer};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn serialize_hresult<S>(value: &Option<u32>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(hr) => serializer.serialize_some(&format!("0x{hr:08X}")),
        None => serializer.serialize_none(),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn is_valid(&self) -> bool {
        self.width() > 0 && self.height() > 0
    }
}

impl From<windows::Win32::Foundation::RECT> for Rect {
    fn from(r: windows::Win32::Foundation::RECT) -> Self {
        Self {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CropRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Pad {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ErrorInfo {
    pub message: String,
    #[serde(rename = "where")]
    pub where_: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_hresult"
    )]
    pub hresult: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win32_error: Option<u32>,
}

impl ErrorInfo {
    pub fn new(message: impl Into<String>, where_: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            where_: where_.into(),
            hresult: None,
            win32_error: None,
        }
    }
    pub fn with_hresult(message: impl Into<String>, where_: impl Into<String>, hr: u32) -> Self {
        Self {
            hresult: Some(hr),
            ..Self::new(message, where_)
        }
    }
    pub fn with_win32(message: impl Into<String>, where_: impl Into<String>, code: u32) -> Self {
        Self {
            win32_error: Some(code),
            ..Self::new(message, where_)
        }
    }
}

impl std::fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.where_)?;
        if let Some(hr) = self.hresult {
            write!(f, " hresult=0x{hr:08X}")?;
        }
        if let Some(code) = self.win32_error {
            write!(f, " win32_error={code}")?;
        }
        Ok(())
    }
}

/// BGRA image. `row_pitch` is always `width * 4` once stored here; capture
/// paths copy row-by-row from the source pitch.
#[derive(Clone, Debug, Default)]
pub struct ImageBuffer {
    pub width: i32,
    pub height: i32,
    pub row_pitch: i32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub bgra: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ImageStats {
    pub black_ratio: f64,
    pub transparent_ratio: f64,
    pub avg_luma: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    #[serde(rename = "class")]
    pub class_name: String,
    pub rect: Rect,
    pub client_rect_screen: Rect,
    pub dwm_frame_rect: Rect,
    pub visible: bool,
    pub iconic: bool,
    pub cloaked: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MonitorInfo {
    pub hmon: isize,
    pub index: i32,
    pub name: String,
    pub desktop: Rect,
    pub primary: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandType {
    Cap,
    ListWindows,
    ListMonitors,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DpiMode {
    Auto,
    PerMonitorV2,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetType {
    Window,
    Screen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropMode {
    None,
    Window,
    Client,
    DwmFrame,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct CommonOptions {
    pub log_dir: String,
    pub log_level: LogLevel,
    pub json: bool,
    pub timeout_ms: i32,
    pub retry: i32,
    pub overwrite: bool,
    pub dpi_mode: DpiMode,
}

impl Default for CommonOptions {
    fn default() -> Self {
        Self {
            log_dir: "./logs".to_string(),
            log_level: LogLevel::Info,
            json: false,
            timeout_ms: 700,
            retry: 0,
            overwrite: false,
            dpi_mode: DpiMode::PerMonitorV2,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TargetWindowQuery {
    pub hwnd: Option<u64>,
    pub pid: Option<i32>,
    pub foreground: bool,
    pub title: Option<String>,
    pub class_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TargetScreenQuery {
    /// Monitor index as a string, or "primary".
    pub monitor: Option<String>,
    pub virtual_screen: bool,
}

#[derive(Clone, Debug)]
pub struct CapOptions {
    pub method: String,
    pub target: TargetType,
    pub out_path: String,
    pub hotkey_enabled: bool,
    pub hotkey_spec: String,
    pub hotkey_modifiers: u32,
    pub hotkey_vk: u32,
    pub window_query: TargetWindowQuery,
    pub screen_query: TargetScreenQuery,
    pub crop_mode: CropMode,
    pub crop_rect: Option<CropRect>,
    pub pad: Pad,
    pub force_alpha_255: bool,
}

impl Default for CapOptions {
    fn default() -> Self {
        Self {
            method: String::new(),
            target: TargetType::Window,
            out_path: String::new(),
            hotkey_enabled: false,
            hotkey_spec: String::new(),
            hotkey_modifiers: 0,
            hotkey_vk: 0,
            window_query: TargetWindowQuery::default(),
            screen_query: TargetScreenQuery::default(),
            crop_mode: CropMode::None,
            crop_rect: None,
            pad: Pad::default(),
            force_alpha_255: false,
        }
    }
}

pub struct CaptureContext<'a> {
    pub cap: &'a CapOptions,
    pub common: &'a CommonOptions,
    pub window: Option<WindowInfo>,
    pub monitor: Option<MonitorInfo>,
    pub capture_rect_screen: Rect,
    pub logger: &'a crate::logging::Logger,
}
