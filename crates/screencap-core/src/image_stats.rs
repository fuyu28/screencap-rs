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
    let mut luma_sum: f64 = 0.0;

    for y in 0..img.height {
        let row_start = (y as usize) * (img.row_pitch as usize);
        let row = &img.bgra[row_start..];
        for x in 0..img.width {
            let base = (x as usize) * 4;
            let b = row[base];
            let g = row[base + 1];
            let r = row[base + 2];
            let a = row[base + 3];
            if r == 0 && g == 0 && b == 0 {
                black += 1;
            }
            if a == 0 {
                transparent += 1;
            }
            luma_sum += 0.2126 * (r as f64) + 0.7152 * (g as f64) + 0.0722 * (b as f64);
        }
    }

    s.black_ratio = black as f64 / pixels as f64;
    s.transparent_ratio = transparent as f64 / pixels as f64;
    s.avg_luma = luma_sum / pixels as f64;
    s
}
