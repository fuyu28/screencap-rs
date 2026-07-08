//! Port of src/crop.cpp.

use crate::types::{CropMode, CropRect, ErrorInfo, ImageBuffer, Pad, Rect, WindowInfo};

fn intersect(a: Rect, b: Rect) -> Rect {
    Rect {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    }
}

/// Resolves the crop rect in screen coordinates: pick the base rect by mode
/// (capture rect / window rect / client rect / dwm frame / manual), expand by
/// pad, then clip to `capture_screen_rect`. Errors if the mode needs a window
/// that is absent or the clipped rect is empty.
pub fn resolve_crop_rect_screen(
    mode: CropMode,
    manual: Option<CropRect>,
    window: Option<&WindowInfo>,
    capture_screen_rect: Rect,
    pad: Pad,
) -> Result<Rect, ErrorInfo> {
    let mut base = match mode {
        CropMode::None => capture_screen_rect,
        CropMode::Window => match window {
            Some(w) => w.rect,
            None => {
                return Err(ErrorInfo::new(
                    "crop window requested but no window target",
                    "ResolveCropRectScreen",
                ))
            }
        },
        CropMode::Client => match window {
            Some(w) => w.client_rect_screen,
            None => {
                return Err(ErrorInfo::new(
                    "crop client requested but no window target",
                    "ResolveCropRectScreen",
                ))
            }
        },
        CropMode::DwmFrame => match window {
            Some(w) => w.dwm_frame_rect,
            None => {
                return Err(ErrorInfo::new(
                    "crop dwm-frame requested but no window target",
                    "ResolveCropRectScreen",
                ))
            }
        },
        CropMode::Manual => match manual {
            Some(m) => Rect {
                left: m.x,
                top: m.y,
                right: m.x + m.w,
                bottom: m.y + m.h,
            },
            None => {
                return Err(ErrorInfo::new(
                    "manual crop missing rect",
                    "ResolveCropRectScreen",
                ))
            }
        },
    };

    base.left -= pad.l;
    base.top -= pad.t;
    base.right += pad.r;
    base.bottom += pad.b;

    let clipped = intersect(base, capture_screen_rect);
    if !clipped.is_valid() {
        return Err(ErrorInfo::new(
            "crop rect is empty after intersection",
            "ResolveCropRectScreen",
        ));
    }
    Ok(clipped)
}

/// Crops `img` in place to the intersection of `crop_screen_rect` and the
/// image's own screen rect. Errors if they do not overlap.
pub fn crop_image_in_place(crop_screen_rect: Rect, img: &mut ImageBuffer) -> Result<(), ErrorInfo> {
    let img_rect = Rect {
        left: img.origin_x,
        top: img.origin_y,
        right: img.origin_x + img.width,
        bottom: img.origin_y + img.height,
    };
    let c = intersect(crop_screen_rect, img_rect);
    if !c.is_valid() {
        return Err(ErrorInfo::new(
            "crop does not overlap image",
            "CropImageInPlace",
        ));
    }

    let x0 = (c.left - img.origin_x) as usize;
    let y0 = (c.top - img.origin_y) as usize;
    let nw = c.width();
    let nh = c.height();

    let mut out = vec![0u8; (nw as usize) * (nh as usize) * 4];
    for y in 0..nh as usize {
        let src_start = (y0 + y) * (img.row_pitch as usize) + x0 * 4;
        let src = &img.bgra[src_start..src_start + (nw as usize) * 4];
        let dst_start = y * (nw as usize) * 4;
        out[dst_start..dst_start + (nw as usize) * 4].copy_from_slice(src);
    }

    img.width = nw;
    img.height = nh;
    img.row_pitch = nw * 4;
    img.origin_x = c.left;
    img.origin_y = c.top;
    img.bgra = out;
    Ok(())
}
