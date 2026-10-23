//! decode_photo on the sample files in tests/sample-images/.

#![cfg(any(feature = "raw", feature = "heic"))]

use std::path::PathBuf;

use image::GenericImageView;
use image_proc::decode::decode_photo;

fn sample(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/sample-images")
        .join(name)
}

#[cfg(feature = "heic")]
#[test]
fn decode_heic_returns_correct_dimensions() {
    // 64x48 gradient, generated via pillow-heif.
    let img = decode_photo(&sample("003.heic"), ".heic", 64).unwrap();
    assert_eq!(img.dimensions(), (64, 48));
}

// 004.dng: Sigma fp DNG, 16:9, CC0, from raw.pixls.us
// ("Sigma - fp - 8bit (16:9).DNG", sha256 80ae0b8f…ad0c).

#[cfg(feature = "raw")]
#[test]
fn decode_raw_dng_for_thumbnail() {
    let img = decode_photo(&sample("004.dng"), ".DNG", 256).unwrap();
    let (w, h) = img.dimensions();
    assert!(w >= 256, "long edge {w} smaller than requested");
    assert_eq!((w * 9 + h / 2) / h, 16, "expected 16:9, got {w}x{h}");
}

#[cfg(feature = "raw")]
#[test]
fn decode_raw_develops_when_embedded_jpeg_too_small() {
    // No embedded JPEG is this large, so this forces the develop path.
    let img = decode_photo(&sample("004.dng"), ".dng", u32::MAX).unwrap();
    let (w, h) = img.dimensions();
    assert_eq!((w * 9 + h / 2) / h, 16, "expected 16:9, got {w}x{h}");
    // Developed output is not black: auto exposure ran.
    let mean = img.to_luma8().pixels().map(|p| p[0] as u64).sum::<u64>() / (w * h) as u64;
    assert!(mean > 40, "developed image too dark (mean luma {mean})");
}

#[cfg(feature = "raw")]
#[test]
fn decode_raw_rejects_non_raw_file_without_panicking() {
    let err = decode_photo(&sample("001.jpg"), ".nef", 256).unwrap_err();
    assert!(err.contains("failed to decode RAW"), "{err}");
}

#[cfg(feature = "raw")]
#[test]
fn decode_raw_missing_file_is_an_error() {
    let err = decode_photo(&PathBuf::from("/nonexistent/x.cr2"), ".cr2", 256).unwrap_err();
    assert!(err.contains("failed to decode RAW"), "{err}");
}
