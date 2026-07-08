//! Port of src/capture_dxgi.cpp (DXGI Output Duplication).
//! Methods: dxgi-monitor, dxgi-window (monitor resolved from ctx.monitor or
//! the window's nearest monitor).

use crate::types::{CaptureContext, ErrorInfo, ImageBuffer};

/// On success also returns (adapter_index, output_index) for logging.
pub fn capture_with_dxgi(_ctx: &CaptureContext) -> Result<(ImageBuffer, i32, i32), ErrorInfo> {
    todo!("port CaptureWithDxgi")
}
