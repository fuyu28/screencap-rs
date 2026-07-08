//! Port of src/capture_wgc.cpp (Windows.Graphics.Capture).
//! Methods: wgc-window / wgc-window2 (window target), wgc-monitor /
//! wgc-monitor2 (monitor target). Crops to frame ContentSize, retries up to 5
//! frames until one passes the usable-frame heuristic
//! (transparent_ratio < 0.98 && black_ratio < 0.98).

use crate::types::{CaptureContext, ErrorInfo, ImageBuffer};

pub fn capture_with_wgc(_ctx: &CaptureContext) -> Result<ImageBuffer, ErrorInfo> {
    todo!("port CaptureWithWgc")
}
