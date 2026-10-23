//! Photo decoding: picks a decoder from the file extension.
//!
//! RAW and HEIC decoders are compile-time features. When a feature is off, its
//! extensions return a "rebuild with --features …" error instead.

use std::path::Path;

use image::DynamicImage;

#[cfg(feature = "heic")]
mod heic;
#[cfg(feature = "raw")]
mod raw;

#[cfg(feature = "heic")]
use heic::decode_heic;
#[cfg(feature = "raw")]
use raw::decode_raw;

/// Decodes a photo file. `min_long_edge` is the smallest long edge the caller
/// needs; RAW decoding uses it to choose between the embedded JPEG and a full
/// RAW decode.
pub fn decode_photo(path: &Path, ext: &str, min_long_edge: u32) -> Result<DynamicImage, String> {
    match ext.to_lowercase().as_str() {
        ".cr2" | ".cr3" | ".nef" | ".arw" | ".dng" | ".raf" | ".orf" | ".rw2" | ".pef" => {
            decode_raw(path, min_long_edge)
        }
        ".heic" | ".heif" => decode_heic(path),
        _ => image::open(path).map_err(|e| format!("failed to open {}: {e}", path.display())),
    }
}

#[cfg(not(feature = "raw"))]
fn decode_raw(path: &Path, _min_long_edge: u32) -> Result<DynamicImage, String> {
    Err(format!(
        "RAW format not supported; rebuild with --features raw: {}",
        path.display()
    ))
}

#[cfg(not(feature = "heic"))]
fn decode_heic(path: &Path) -> Result<DynamicImage, String> {
    Err(format!(
        "HEIC/HEIF format not supported; rebuild with --features heic: {}",
        path.display()
    ))
}
