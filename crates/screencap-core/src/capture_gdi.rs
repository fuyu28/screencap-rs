//! Port of src/capture_gdi.cpp.
//! Methods: gdi-printwindow, gdi-bitblt-client, gdi-bitblt-windowdc,
//! gdi-bitblt-screen.

use crate::types::{CaptureContext, ErrorInfo, ImageBuffer};

pub fn capture_with_gdi(_ctx: &CaptureContext) -> Result<ImageBuffer, ErrorInfo> {
    todo!("port CaptureWithGdi")
}
