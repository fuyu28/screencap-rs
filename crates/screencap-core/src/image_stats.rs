//! Port of src/image_stats.cpp.

use crate::types::{ImageBuffer, ImageStats};

/// black_ratio = pixels with r==g==b==0, transparent_ratio = pixels with a==0,
/// avg_luma = mean of 0.2126r + 0.7152g + 0.0722b. Zero stats for empty images.
pub fn compute_image_stats(_img: &ImageBuffer) -> ImageStats {
    todo!("port ComputeImageStats")
}
