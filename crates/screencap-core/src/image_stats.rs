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
