//! Port of src/crop.cpp.

use crate::types::{CropMode, CropRect, ErrorInfo, ImageBuffer, Pad, Rect, WindowInfo};

/// Resolves the crop rect in screen coordinates: pick the base rect by mode
/// (capture rect / window rect / client rect / dwm frame / manual), expand by
/// pad, then clip to `capture_screen_rect`. Errors if the mode needs a window
/// that is absent or the clipped rect is empty.
pub fn resolve_crop_rect_screen(
    _mode: CropMode,
    _manual: Option<CropRect>,
    _window: Option<&WindowInfo>,
    _capture_screen_rect: Rect,
    _pad: Pad,
) -> Result<Rect, ErrorInfo> {
    todo!("port ResolveCropRectScreen")
}

/// Crops `img` in place to the intersection of `crop_screen_rect` and the
/// image's own screen rect. Errors if they do not overlap.
pub fn crop_image_in_place(_crop_screen_rect: Rect, _img: &mut ImageBuffer) -> Result<(), ErrorInfo> {
    todo!("port CropImageInPlace")
}
