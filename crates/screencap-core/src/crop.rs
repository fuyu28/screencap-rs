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
    let need_window = |what: &str| {
        window.ok_or_else(|| {
            ErrorInfo::new(
                format!("crop {what} requested but no window target"),
                "ResolveCropRectScreen",
            )
        })
    };

    let mut base = match mode {
        CropMode::None => capture_screen_rect,
        CropMode::Window => need_window("window")?.rect,
        CropMode::Client => need_window("client")?.client_rect_screen,
        CropMode::DwmFrame => need_window("dwm-frame")?.dwm_frame_rect,
        CropMode::Manual => match manual {
            Some(m) => Rect {
                left: m.x,
                top: m.y,
                right: m.x.saturating_add(m.w),
                bottom: m.y.saturating_add(m.h),
            },
            None => {
                return Err(ErrorInfo::new(
                    "manual crop missing rect",
                    "ResolveCropRectScreen",
                ));
            }
        },
    };

    base.left = base.left.saturating_sub(pad.l);
    base.top = base.top.saturating_sub(pad.t);
    base.right = base.right.saturating_add(pad.r);
    base.bottom = base.bottom.saturating_add(pad.b);

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

    if c == img_rect && img.row_pitch == img.width * 4 {
        return Ok(());
    }

    let x0 = (c.left - img.origin_x) as usize;
    let y0 = (c.top - img.origin_y) as usize;
    let nw = c.width();
    let nh = c.height();

    let mut out = Vec::with_capacity((nw as usize) * (nh as usize) * 4);
    for y in 0..nh as usize {
        let src_start = (y0 + y) * (img.row_pitch as usize) + x0 * 4;
        let src = &img.bgra[src_start..src_start + (nw as usize) * 4];
        out.extend_from_slice(src);
    }

    img.width = nw;
    img.height = nh;
    img.row_pitch = nw * 4;
    img.origin_x = c.left;
    img.origin_y = c.top;
    img.bgra = out;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    fn window_with_rects(rect: Rect, client: Rect, dwm: Rect) -> WindowInfo {
        WindowInfo {
            hwnd: 1,
            pid: 2,
            title: String::new(),
            class_name: String::new(),
            rect,
            client_rect_screen: client,
            dwm_frame_rect: dwm,
            visible: true,
            iconic: false,
            cloaked: false,
        }
    }

    /// Builds a tightly-packed (row_pitch == width*4) BGRA buffer where each
    /// pixel's bytes encode its own coordinate: [b=x, g=y, r=0, a=255].
    fn coord_buffer(width: i32, height: i32, origin_x: i32, origin_y: i32) -> ImageBuffer {
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                bgra.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        ImageBuffer {
            width,
            height,
            row_pitch: width * 4,
            origin_x,
            origin_y,
            bgra,
        }
    }

    #[test]
    fn resolve_none_returns_capture_rect() {
        let cap = rect(0, 0, 100, 80);
        let out =
            resolve_crop_rect_screen(CropMode::None, None, None, cap, Pad::default()).unwrap();
        assert_eq!(out, cap);
    }

    #[test]
    fn resolve_none_pad_cannot_exceed_capture_rect() {
        // Pad expands the base, but the final intersection clips back to the
        // capture rect, so padding a full-screen None crop is a no-op.
        let cap = rect(0, 0, 100, 80);
        let pad = Pad {
            l: 10,
            t: 10,
            r: 10,
            b: 10,
        };
        let out = resolve_crop_rect_screen(CropMode::None, None, None, cap, pad).unwrap();
        assert_eq!(out, cap);
    }

    #[test]
    fn resolve_window_mode_clips_to_capture() {
        let cap = rect(0, 0, 100, 80);
        let win = window_with_rects(rect(-20, -10, 50, 40), Rect::default(), Rect::default());
        let out = resolve_crop_rect_screen(CropMode::Window, None, Some(&win), cap, Pad::default())
            .unwrap();
        // Window rect intersected with capture rect.
        assert_eq!(out, rect(0, 0, 50, 40));
    }

    #[test]
    fn resolve_window_mode_requires_window() {
        let cap = rect(0, 0, 100, 80);
        let err = resolve_crop_rect_screen(CropMode::Window, None, None, cap, Pad::default())
            .unwrap_err();
        assert_eq!(err.where_, "ResolveCropRectScreen");
        assert!(err.message.contains("window"));
    }

    #[test]
    fn resolve_client_mode_uses_client_rect() {
        let cap = rect(0, 0, 100, 80);
        let win = window_with_rects(rect(0, 0, 100, 80), rect(10, 10, 60, 50), Rect::default());
        let out = resolve_crop_rect_screen(CropMode::Client, None, Some(&win), cap, Pad::default())
            .unwrap();
        assert_eq!(out, rect(10, 10, 60, 50));
    }

    #[test]
    fn resolve_dwm_frame_mode_uses_dwm_rect() {
        let cap = rect(0, 0, 100, 80);
        let win = window_with_rects(rect(0, 0, 100, 80), Rect::default(), rect(5, 6, 40, 45));
        let out =
            resolve_crop_rect_screen(CropMode::DwmFrame, None, Some(&win), cap, Pad::default())
                .unwrap();
        assert_eq!(out, rect(5, 6, 40, 45));
    }

    #[test]
    fn resolve_manual_mode_builds_rect_from_xywh() {
        let cap = rect(0, 0, 100, 80);
        let manual = CropRect {
            x: 10,
            y: 20,
            w: 30,
            h: 15,
        };
        let out =
            resolve_crop_rect_screen(CropMode::Manual, Some(manual), None, cap, Pad::default())
                .unwrap();
        assert_eq!(out, rect(10, 20, 40, 35));
    }

    #[test]
    fn resolve_manual_mode_requires_rect() {
        let cap = rect(0, 0, 100, 80);
        let err = resolve_crop_rect_screen(CropMode::Manual, None, None, cap, Pad::default())
            .unwrap_err();
        assert!(err.message.contains("manual"));
    }

    #[test]
    fn resolve_pad_expands_within_capture() {
        let cap = rect(0, 0, 100, 80);
        let win = window_with_rects(rect(30, 30, 50, 50), Rect::default(), Rect::default());
        let pad = Pad {
            l: 5,
            t: 4,
            r: 3,
            b: 2,
        };
        let out = resolve_crop_rect_screen(CropMode::Window, None, Some(&win), cap, pad).unwrap();
        assert_eq!(out, rect(25, 26, 53, 52));
    }

    #[test]
    fn resolve_errors_when_clip_is_empty() {
        let cap = rect(0, 0, 100, 80);
        // Window rect lies entirely to the right of the capture rect.
        let win = window_with_rects(rect(200, 200, 300, 300), Rect::default(), Rect::default());
        let err = resolve_crop_rect_screen(CropMode::Window, None, Some(&win), cap, Pad::default())
            .unwrap_err();
        assert!(err.message.contains("empty"));
    }

    #[test]
    fn crop_in_place_extracts_subregion() {
        let mut img = coord_buffer(4, 3, 0, 0);
        crop_image_in_place(rect(1, 1, 3, 3), &mut img).unwrap();

        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        assert_eq!(img.row_pitch, 8);
        assert_eq!(img.origin_x, 1);
        assert_eq!(img.origin_y, 1);
        // Pixels (1,1),(2,1),(1,2),(2,2) with encoding [x, y, 0, 255].
        assert_eq!(
            img.bgra,
            vec![1, 1, 0, 255, 2, 1, 0, 255, 1, 2, 0, 255, 2, 2, 0, 255]
        );
    }

    #[test]
    fn crop_in_place_respects_nonzero_origin() {
        // Image occupies screen rect (10,10)-(14,13); crop the middle column.
        let mut img = coord_buffer(4, 3, 10, 10);
        crop_image_in_place(rect(11, 10, 13, 12), &mut img).unwrap();
        assert_eq!(img.origin_x, 11);
        assert_eq!(img.origin_y, 10);
        assert_eq!(img.width, 2);
        assert_eq!(img.height, 2);
        // Local x0 = 11-10 = 1, y0 = 0. Row 0: pixels (1,0),(2,0); row1 (1,1),(2,1).
        assert_eq!(
            img.bgra,
            vec![1, 0, 0, 255, 2, 0, 0, 255, 1, 1, 0, 255, 2, 1, 0, 255]
        );
    }

    #[test]
    fn crop_in_place_full_rect_tightly_packed_is_noop() {
        let mut img = coord_buffer(4, 3, 0, 0);
        let original = img.bgra.clone();
        crop_image_in_place(rect(0, 0, 4, 3), &mut img).unwrap();
        assert_eq!(img.bgra, original);
        assert_eq!(img.row_pitch, 16);
    }

    #[test]
    fn crop_in_place_full_rect_padded_pitch_repacks() {
        // row_pitch has 8 bytes of padding per row; a "full rect" crop still
        // rebuilds the buffer to a tight pitch.
        let width = 2;
        let height = 2;
        let row_pitch = width * 4 + 8;
        let mut bgra = Vec::new();
        for y in 0..height {
            for x in 0..width {
                bgra.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
            bgra.extend_from_slice(&[0xFF; 8]); // padding bytes
        }
        let mut img = ImageBuffer {
            width,
            height,
            row_pitch,
            origin_x: 0,
            origin_y: 0,
            bgra,
        };
        crop_image_in_place(rect(0, 0, 2, 2), &mut img).unwrap();
        assert_eq!(img.row_pitch, 8);
        assert_eq!(
            img.bgra,
            vec![0, 0, 0, 255, 1, 0, 0, 255, 0, 1, 0, 255, 1, 1, 0, 255]
        );
    }

    #[test]
    fn crop_in_place_no_overlap_errors() {
        let mut img = coord_buffer(4, 3, 0, 0);
        let err = crop_image_in_place(rect(100, 100, 110, 110), &mut img).unwrap_err();
        assert_eq!(err.where_, "CropImageInPlace");
    }
}
