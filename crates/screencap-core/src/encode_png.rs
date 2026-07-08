//! Port of src/encode_wic_png.cpp: PNG encode via WIC, 32bpp BGRA.

use crate::types::{ErrorInfo, ImageBuffer};

/// Refuses to overwrite an existing file unless `overwrite`
/// ("output exists (use --overwrite)"). Handles COM init/uninit internally
/// (tolerates RPC_E_CHANGED_MODE).
pub fn save_png_wic(_img: &ImageBuffer, _out_path: &str, _overwrite: bool) -> Result<(), ErrorInfo> {
    todo!("port SavePngWic")
}
