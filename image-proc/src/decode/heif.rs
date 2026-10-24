//! HEIF-container decoding through libheif (`heic` feature): HEIC (HEVC) and
//! AVIF (AV1). AVIF needs libheif built with an AV1 decoder (aom or dav1d).

use std::path::Path;

use image::{DynamicImage, RgbImage};

/// Decodes the primary image. libheif applies the file's rotation and mirror
/// transforms, so the result is upright; higher bit depths are converted to
/// 8-bit RGB.
pub fn decode_heif(path: &Path) -> Result<DynamicImage, String> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

    let path_str = path
        .to_str()
        .ok_or_else(|| format!("non-UTF8 path: {}", path.display()))?;
    let ctx = HeifContext::read_from_file(path_str)
        .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let handle = ctx
        .primary_image_handle()
        .map_err(|e| format!("no primary image in {}: {e}", path.display()))?;
    let lib_heif = LibHeif::new();
    let image = lib_heif
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
        .map_err(|e| format!("failed to decode {}: {e}", path.display()))?;
    let plane = image
        .planes()
        .interleaved
        .ok_or_else(|| format!("no interleaved RGB plane in {}", path.display()))?;

    let width = plane.width;
    let height = plane.height;
    let row_bytes = width as usize * 3;
    let mut buf = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * plane.stride;
        buf.extend_from_slice(&plane.data[start..start + row_bytes]);
    }

    let rgb_image = RgbImage::from_raw(width, height, buf)
        .ok_or_else(|| format!("pixel buffer size mismatch decoding {}", path.display()))?;
    Ok(DynamicImage::ImageRgb8(rgb_image))
}
