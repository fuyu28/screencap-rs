use crate::types::{ImageBuffer, ImageStats};

/// black_ratio = pixels with r==g==b==0, transparent_ratio = pixels with a==0,
/// avg_luma = mean of 0.2126r + 0.7152g + 0.0722b. Zero stats for empty images.
pub fn compute_image_stats(img: &ImageBuffer) -> ImageStats {
    let mut s = ImageStats::default();
    if img.width <= 0 || img.height <= 0 || img.bgra.is_empty() {
        return s;
    }

    let pixels = (img.width as u64) * (img.height as u64);
    let mut black: u64 = 0;
    let mut transparent: u64 = 0;
    let mut r_sum: u64 = 0;
    let mut g_sum: u64 = 0;
    let mut b_sum: u64 = 0;
    let row_len = (img.width as usize) * 4;

    for y in 0..img.height {
        let row_start = (y as usize) * (img.row_pitch as usize);
        let row = &img.bgra[row_start..row_start + row_len];
        for px in row.chunks_exact(4) {
            let b = px[0];
            let g = px[1];
            let r = px[2];
            let a = px[3];
            if r == 0 && g == 0 && b == 0 {
                black += 1;
            }
            if a == 0 {
                transparent += 1;
            }
            r_sum += r as u64;
            g_sum += g as u64;
            b_sum += b as u64;
        }
    }

    s.black_ratio = black as f64 / pixels as f64;
    s.transparent_ratio = transparent as f64 / pixels as f64;
    s.avg_luma = (0.2126 * (r_sum as f64) + 0.7152 * (g_sum as f64) + 0.0722 * (b_sum as f64))
        / pixels as f64;
    s
}

/// Lighter variant of `compute_image_stats` for callers that only need
/// (black_ratio, transparent_ratio) and don't want the avg_luma sums
/// accumulated. Zero ratios for empty images.
pub fn compute_frame_ratios(img: &ImageBuffer) -> (f64, f64) {
    if img.width <= 0 || img.height <= 0 || img.bgra.is_empty() {
        return (0.0, 0.0);
    }

    let pixels = (img.width as u64) * (img.height as u64);
    let mut black: u64 = 0;
    let mut transparent: u64 = 0;
    let row_len = (img.width as usize) * 4;

    for y in 0..img.height {
        let row_start = (y as usize) * (img.row_pitch as usize);
        let row = &img.bgra[row_start..row_start + row_len];
        for px in row.chunks_exact(4) {
            let b = px[0];
            let g = px[1];
            let r = px[2];
            let a = px[3];
            if r == 0 && g == 0 && b == 0 {
                black += 1;
            }
            if a == 0 {
                transparent += 1;
            }
        }
    }

    (
        black as f64 / pixels as f64,
        transparent as f64 / pixels as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tightly-packed BGRA buffer that repeats `pixel` for every cell.
    fn solid(width: i32, height: i32, pixel: [u8; 4]) -> ImageBuffer {
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            bgra.extend_from_slice(&pixel);
        }
        ImageBuffer {
            width,
            height,
            row_pitch: width * 4,
            origin_x: 0,
            origin_y: 0,
            bgra,
        }
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn all_black_opaque() {
        let img = solid(4, 4, [0, 0, 0, 255]);
        let s = compute_image_stats(&img);
        approx(s.black_ratio, 1.0);
        approx(s.transparent_ratio, 0.0);
        approx(s.avg_luma, 0.0);
    }

    #[test]
    fn all_white_opaque() {
        let img = solid(4, 4, [255, 255, 255, 255]);
        let s = compute_image_stats(&img);
        approx(s.black_ratio, 0.0);
        approx(s.transparent_ratio, 0.0);
        // 0.2126*255 + 0.7152*255 + 0.0722*255 == 255.
        approx(s.avg_luma, 255.0);
    }

    #[test]
    fn all_transparent_black_counts_both() {
        // r==g==b==0 counts as black; a==0 counts as transparent.
        let img = solid(2, 2, [0, 0, 0, 0]);
        let s = compute_image_stats(&img);
        approx(s.black_ratio, 1.0);
        approx(s.transparent_ratio, 1.0);
    }

    #[test]
    fn half_black_half_white() {
        // Two black + two white pixels.
        let mut img = solid(2, 2, [0, 0, 0, 255]);
        // Overwrite the last two pixels with white.
        for i in 2..4 {
            let base = i * 4;
            img.bgra[base..base + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let s = compute_image_stats(&img);
        approx(s.black_ratio, 0.5);
        approx(s.transparent_ratio, 0.0);
        approx(s.avg_luma, 127.5);
    }

    #[test]
    fn luma_uses_bt709_weights() {
        // Pure green pixel: b=0, g=255, r=0 -> luma = 0.7152*255.
        let img = solid(1, 1, [0, 255, 0, 255]);
        let s = compute_image_stats(&img);
        approx(s.avg_luma, 0.7152 * 255.0);
        approx(s.black_ratio, 0.0);
    }

    #[test]
    fn empty_image_is_zeroed() {
        let img = ImageBuffer::default();
        let s = compute_image_stats(&img);
        approx(s.black_ratio, 0.0);
        approx(s.transparent_ratio, 0.0);
        approx(s.avg_luma, 0.0);
        assert_eq!(compute_frame_ratios(&img), (0.0, 0.0));
    }

    #[test]
    fn stats_ignore_row_pitch_padding() {
        // 2x2 image with 8 padding bytes per row; padding must not be counted.
        let width = 2;
        let height = 2;
        let row_pitch = width * 4 + 8;
        let mut bgra = Vec::new();
        for _ in 0..height {
            for _ in 0..width {
                bgra.extend_from_slice(&[0, 0, 0, 255]); // black, opaque
            }
            // Non-black, transparent padding that would skew stats if counted.
            bgra.extend_from_slice(&[255, 255, 255, 0]);
            bgra.extend_from_slice(&[255, 255, 255, 0]);
        }
        let img = ImageBuffer {
            width,
            height,
            row_pitch,
            origin_x: 0,
            origin_y: 0,
            bgra,
        };
        let s = compute_image_stats(&img);
        approx(s.black_ratio, 1.0);
        approx(s.transparent_ratio, 0.0);
        approx(s.avg_luma, 0.0);
    }

    #[test]
    fn frame_ratios_match_full_stats() {
        let mut img = solid(2, 2, [0, 0, 0, 0]);
        // Make one pixel opaque white so ratios are strictly between 0 and 1.
        img.bgra[0..4].copy_from_slice(&[255, 255, 255, 255]);
        let full = compute_image_stats(&img);
        let (black, transparent) = compute_frame_ratios(&img);
        approx(black, full.black_ratio);
        approx(transparent, full.transparent_ratio);
        approx(black, 0.75);
        approx(transparent, 0.75);
    }
}
