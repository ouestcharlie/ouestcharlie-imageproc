//! Photo decoding: picks a decoder from the file extension.
//!
//! RAW and HEIF decoders are compile-time features. When a feature is off, its
//! extensions return a "rebuild with --features …" error instead, and are left
//! out of [`supported_extensions`].

use std::path::Path;

use image::DynamicImage;

use crate::transform::apply_orientation;

#[cfg(feature = "heic")]
mod heif;
mod jxl;
#[cfg(feature = "raw")]
mod raw;

#[cfg(feature = "heic")]
use heif::decode_heif;
use jxl::decode_jxl;
#[cfg(feature = "raw")]
use raw::decode_raw;

/// Decoded by the `image` crate (always built in).
const STANDARD_EXTENSIONS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".tif", ".tiff", ".webp", ".gif", ".bmp",
];
/// Decoded by jxl-oxide (always built in).
const JXL_EXTENSIONS: &[&str] = &[".jxl"];
/// Decoded by libheif (`heic` feature): the HEIF container, with HEVC or AV1.
const HEIF_EXTENSIONS: &[&str] = &[".heic", ".heif", ".hif", ".avif"];
/// Decoded by rawler (`raw` feature). Formats rawler 0.7.2 cannot decode
/// reliably are left out on purpose: see "RAW decoding" in the LLD.
#[rustfmt::skip]
const RAW_EXTENSIONS: &[&str] = &[
    // Current camera makers
    ".cr2", ".cr3", ".crw", ".nef", ".nrw", ".arw", ".srf", ".sr2", ".dng", ".raf", ".orf",
    ".rw2", ".rwl", ".pef", ".srw", ".erf",
    // Medium format
    ".3fr", ".fff", ".iiq", ".mos",
];

/// Lowercase extensions (with the dot) this build can decode.
pub fn supported_extensions() -> Vec<&'static str> {
    let mut exts: Vec<&str> = STANDARD_EXTENSIONS.to_vec();
    exts.extend_from_slice(JXL_EXTENSIONS);
    if cfg!(feature = "heic") {
        exts.extend_from_slice(HEIF_EXTENSIONS);
    }
    if cfg!(feature = "raw") {
        exts.extend_from_slice(RAW_EXTENSIONS);
    }
    exts
}

/// Decodes a photo and turns it upright.
///
/// HEIF (HEIC, AVIF) and JPEG XL decoders already apply the file's own rotation,
/// so `orientation` is ignored for them; applying it again would rotate twice.
/// For every other format the EXIF `orientation` is applied here.
pub fn decode_oriented(
    path: &Path,
    ext: &str,
    orientation: Option<u8>,
    min_long_edge: u32,
) -> Result<DynamicImage, String> {
    let img = decode_photo(path, ext, min_long_edge)?;
    let ext = ext.to_lowercase();
    let upright = HEIF_EXTENSIONS.contains(&ext.as_str()) || JXL_EXTENSIONS.contains(&ext.as_str());
    Ok(if upright {
        img
    } else {
        apply_orientation(img, orientation)
    })
}

/// Decodes a photo file without applying any EXIF orientation. `min_long_edge`
/// is the smallest long edge the caller needs; RAW decoding uses it to choose
/// between the embedded JPEG and a full RAW decode.
pub fn decode_photo(path: &Path, ext: &str, min_long_edge: u32) -> Result<DynamicImage, String> {
    let ext = ext.to_lowercase();
    let ext = ext.as_str();
    if RAW_EXTENSIONS.contains(&ext) {
        decode_raw(path, min_long_edge)
    } else if HEIF_EXTENSIONS.contains(&ext) {
        decode_heif(path)
    } else if JXL_EXTENSIONS.contains(&ext) {
        decode_jxl(path)
    } else if STANDARD_EXTENSIONS.contains(&ext) {
        decode_standard(path)
    } else {
        Err(format!("unsupported format {ext}: {}", path.display()))
    }
}

/// JPEG, PNG, TIFF, WebP, GIF, BMP. The format is detected from the content, so
/// a file with the wrong extension still decodes. Multi-page TIFF and animated
/// WebP/GIF give their first image.
fn decode_standard(path: &Path) -> Result<DynamicImage, String> {
    let fail = |e: &dyn std::fmt::Display| format!("failed to open {}: {e}", path.display());
    image::ImageReader::open(path)
        .map_err(|e| fail(&e))?
        .with_guessed_format()
        .map_err(|e| fail(&e))?
        .decode()
        .map_err(|e| fail(&e))
}

#[cfg(not(feature = "raw"))]
fn decode_raw(path: &Path, _min_long_edge: u32) -> Result<DynamicImage, String> {
    Err(format!(
        "RAW format not supported; rebuild with --features raw: {}",
        path.display()
    ))
}

#[cfg(not(feature = "heic"))]
fn decode_heif(path: &Path) -> Result<DynamicImage, String> {
    Err(format!(
        "HEIC/HEIF/AVIF format not supported; rebuild with --features heic: {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_are_lowercase_and_dotted() {
        for ext in supported_extensions() {
            assert!(ext.starts_with('.'), "{ext}");
            assert_eq!(ext, ext.to_lowercase(), "{ext}");
        }
    }

    #[test]
    fn supported_extensions_follow_features() {
        let exts = supported_extensions();
        assert!(exts.contains(&".jpg") && exts.contains(&".jxl"));
        assert_eq!(exts.contains(&".avif"), cfg!(feature = "heic"));
        assert_eq!(exts.contains(&".nef"), cfg!(feature = "raw"));
    }

    #[test]
    fn unknown_extension_is_an_error() {
        let err = decode_photo(Path::new("/tmp/x.ico"), ".ico", 256).unwrap_err();
        assert!(err.contains("unsupported format .ico"), "{err}");
    }
}
