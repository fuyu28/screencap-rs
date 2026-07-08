//! Windows screenshot capture library.

pub mod capture_dxgi;
pub mod capture_gdi;
pub mod capture_wgc;
pub mod crop;
mod d3d11_copy;
pub mod encode_png;
pub mod image_stats;
pub mod logging;
pub mod monitor_enum;
pub mod types;
pub mod util;
pub mod window_enum;

pub use types::*;
