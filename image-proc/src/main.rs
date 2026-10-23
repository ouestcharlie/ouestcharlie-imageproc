//! image-proc — image processing coprocessor for OuEstCharlie.
//!
//! Reads a JSON object from stdin and dispatches to one of two commands
//! based on the shape of the input:
//!
//! # Command: avif_grid (when "photos" plural field is present)
//!
//! Decodes photos and assembles them into an AVIF grid container.
//!
//! ```json
//! {
//!   "photos": [
//!     { "path": "/tmp/staged.jpg", "ext": ".jpg", "orientation": 6, "content_hash": "Kf3QzA2nBcR8xYvLm1P9w"},
//!     ...
//!   ],
//!   "tile_size": 256,
//!   "fit": "crop",
//!   "quality": 55,
//!   "output": "/tmp/output.avif"
//! }
//! ```
//! `fit` is `"crop"` (center-crop to square) or `"pad"` (letterbox with black).
//!
//! Output:
//! ```json
//! {"cols": 32, "rows": 4, "tileSize": 256, "photoOrder": ["Kf3QzA2nBcR8xYvLm1P9w", ...]}
//! ```
//!
//! # Command: jpeg_preview (when "photo" singular field is present)
//!
//! Decodes a single photo, applies EXIF orientation, resizes to max_long_edge,
//! and saves as JPEG.
//!
//! ```json
//! {
//!   "photo": { "path": "/tmp/staged.cr2", "ext": ".cr2", "orientation": 6, "content_hash": "Kf3QzA2nBcR8xYvLm1P9w"},
//!   "max_long_edge": 1440,
//!   "quality": 85,
//!   "output": "/tmp/preview.jpg"
//! }
//! ```
//!
//! Output:
//! ```json
//! {"width": 1440, "height": 960}
//! ```
//!
//! # Grid layout (avif_grid)
//!
//! Photos are always arranged in 8-column rows: `cols = 8`,
//! `rows = ceil(n / 8)`.  The last row is padded with blank black tiles.
//!
//! The caller is responsible for ordering photos by content_hash before
//! passing them here, to ensure stable tile indices.

use std::io::{self};
use std::path::{Path, PathBuf};

use image::{DynamicImage, GenericImageView, RgbImage};
use rayon::prelude::*;
use rgb::FromSlice;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// I/O types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PhotoInput {
    path: PathBuf,
    ext: String,
    #[serde(default)]
    orientation: Option<u8>,
    content_hash: String,
}

/// Dispatch by shape: presence of "photos" (plural) → AvifGrid;
/// presence of "photo" (singular) → JpegPreview.
#[derive(Deserialize)]
#[serde(untagged)]
enum Request {
    AvifGrid(AvifGridInput),
    JpegPreview(JpegPreviewInput),
}

#[derive(Deserialize)]
struct AvifGridInput {
    photos: Vec<PhotoInput>,
    tile_size: u32,
    fit: String,
    quality: u8,
    output: PathBuf,
}

#[derive(Deserialize)]
struct JpegPreviewInput {
    photo: PhotoInput,
    max_long_edge: u32,
    quality: u8,
    output: PathBuf,
}

#[derive(Serialize)]
struct AvifGridOutput {
    rows: u32,
    #[serde(rename = "tileSize")]
    tile_size: u32,
    #[serde(rename = "photoOrder")]
    photo_order: Vec<String>,
}

#[derive(Serialize)]
struct JpegPreviewOutput {
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

/// Response envelope: either a successful result or an in-band error.
#[derive(Serialize)]
#[serde(untagged)]
enum Response {
    AvifGrid(AvifGridOutput),
    JpegPreview(JpegPreviewOutput),
    Error { error: String },
}

/// Process one JSON line and return the serialised response line.
/// Extracted from `main` so it can be unit-tested independently.
fn process_line(line: &str, expected_major: u64) -> Response {
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Response::Error {
                error: format!("invalid JSON input: {e}"),
            }
        }
    };
    match value.get("protocol_version").and_then(|v| v.as_u64()) {
        None => Response::Error {
            error: "missing or invalid protocol_version field".to_string(),
        },
        Some(proto) if proto != expected_major => Response::Error {
            error: format!("unsupported protocol version {proto}, expected {expected_major}"),
        },
        Some(_) => match serde_json::from_value::<Request>(value) {
            Err(e) => Response::Error {
                error: format!("invalid request: {e}"),
            },
            Ok(Request::AvifGrid(input)) => match run_avif_grid(input) {
                Ok(out) => Response::AvifGrid(out),
                Err(e) => Response::Error {
                    error: e.to_string(),
                },
            },
            Ok(Request::JpegPreview(input)) => match run_jpeg_preview(input) {
                Ok(out) => Response::JpegPreview(out),
                Err(e) => Response::Error {
                    error: e.to_string(),
                },
            },
        },
    }
}

fn main() {
    use std::io::{BufRead, Write};

    // Support `--version` for version negotiation with the Python toolkit.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("image-proc {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let expected_major: u64 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(1);

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("image-proc: failed to read line: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = process_line(&line, expected_major);
        let json = serde_json::to_string(&response).unwrap();
        if let Err(e) = writeln!(out, "{json}") {
            eprintln!("image-proc: failed to write response: {e}");
            break;
        }
        if let Err(e) = out.flush() {
            eprintln!("image-proc: failed to flush: {e}");
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Command: avif_grid
// ---------------------------------------------------------------------------

fn run_avif_grid(input: AvifGridInput) -> Result<AvifGridOutput, Box<dyn std::error::Error>> {
    let n = input.photos.len();
    if n == 0 {
        return Err("no photos provided".into());
    }

    let tile_size = input.tile_size;
    let fit = input.fit.clone();
    let (cols, rows) = grid_dims(n);
    let total_cells = (cols * rows) as usize;

    // Phase 1: Decode + resize + fit in parallel.
    let mut tiles: Vec<RgbImage> = input
        .photos
        .par_iter()
        .map(|photo| {
            decode_and_prepare(&photo.path, &photo.ext, photo.orientation, tile_size, &fit)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e: String| -> Box<dyn std::error::Error> { e.into() })?;

    // Pad last row with blank black tiles.
    while tiles.len() < total_cells {
        tiles.push(RgbImage::new(tile_size, tile_size));
    }

    // Phase 2: Composite all tiles into a single canvas.
    let total_w = cols * tile_size;
    let total_h = rows * tile_size;
    let mut canvas = RgbImage::new(total_w, total_h);
    for (i, tile) in tiles.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        image::imageops::overlay(
            &mut canvas,
            tile,
            (col * tile_size) as i64,
            (row * tile_size) as i64,
        );
    }

    // Phase 3: Encode as AVIF using ravif (pure Rust, no system deps).
    let pixels: &[rgb::RGB8] = canvas.as_raw().as_rgb();
    let img = ravif::Img::new(pixels, total_w as usize, total_h as usize);
    let encoded = ravif::Encoder::new()
        .with_quality(input.quality as f32)
        .with_speed(6)
        .encode_rgb(img)
        .map_err(|e| format!("AVIF encoding failed: {e}"))?;

    std::fs::write(&input.output, &encoded.avif_file)?;

    let photo_order: Vec<String> = input
        .photos
        .iter()
        .map(|p| p.content_hash.clone())
        .collect();
    Ok(AvifGridOutput {
        rows,
        tile_size,
        photo_order,
    })
}

// ---------------------------------------------------------------------------
// Command: jpeg_preview
// ---------------------------------------------------------------------------

fn run_jpeg_preview(
    input: JpegPreviewInput,
) -> Result<JpegPreviewOutput, Box<dyn std::error::Error>> {
    let photo = &input.photo;
    let img = decode_photo(&photo.path, &photo.ext, input.max_long_edge)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let img = apply_orientation(img, photo.orientation);
    let img = resize_long_edge(img, input.max_long_edge);
    let (width, height) = img.dimensions();

    let file = std::fs::File::create(&input.output)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, input.quality);
    encoder.encode_image(&img.into_rgb8())?;

    Ok(JpegPreviewOutput { width, height })
}

// ---------------------------------------------------------------------------
// Decode + prepare pipeline
// ---------------------------------------------------------------------------

fn decode_and_prepare(
    path: &Path,
    ext: &str,
    orientation: Option<u8>,
    tile_size: u32,
    fit: &str,
) -> Result<RgbImage, String> {
    let img = decode_photo(path, ext, tile_size)?;
    let img = apply_orientation(img, orientation);
    Ok(if fit == "crop" {
        fit_crop(resize_short_edge(img, tile_size), tile_size)
    } else {
        fit_pad(resize_long_edge(img, tile_size), tile_size)
    })
}

/// Decodes a photo file. `min_long_edge` is the smallest long edge the caller
/// needs; RAW decoding uses it to choose between the embedded JPEG and a full
/// RAW decode.
fn decode_photo(path: &Path, ext: &str, min_long_edge: u32) -> Result<DynamicImage, String> {
    match ext.to_lowercase().as_str() {
        ".cr2" | ".cr3" | ".nef" | ".arw" | ".dng" | ".raf" | ".orf" | ".rw2" | ".pef" => {
            decode_raw(path, min_long_edge)
        }
        ".heic" | ".heif" => decode_heic(path),
        _ => image::open(path).map_err(|e| format!("failed to open {}: {e}", path.display())),
    }
}

fn apply_orientation(img: DynamicImage, orientation: Option<u8>) -> DynamicImage {
    match orientation {
        None | Some(1) => img,
        Some(2) => img.fliph(),
        Some(3) => img.rotate180(),
        Some(4) => img.flipv(),
        Some(5) => img.rotate90().fliph(),
        Some(6) => img.rotate90(),
        Some(7) => img.rotate90().flipv(),
        Some(8) => img.rotate270(),
        _ => img,
    }
}

fn resize_short_edge(img: DynamicImage, size: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    let (new_w, new_h) = if w <= h {
        (size, (h as f64 * size as f64 / w as f64).round() as u32)
    } else {
        ((w as f64 * size as f64 / h as f64).round() as u32, size)
    };
    img.resize_exact(
        new_w.max(1),
        new_h.max(1),
        image::imageops::FilterType::Lanczos3,
    )
}

fn resize_long_edge(img: DynamicImage, size: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= size {
        return img;
    }
    let (new_w, new_h) = if w >= h {
        (size, (h as f64 * size as f64 / w as f64).round() as u32)
    } else {
        ((w as f64 * size as f64 / h as f64).round() as u32, size)
    };
    img.resize_exact(
        new_w.max(1),
        new_h.max(1),
        image::imageops::FilterType::Lanczos3,
    )
}

fn fit_crop(img: DynamicImage, size: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    img.crop_imm(
        w.saturating_sub(size) / 2,
        h.saturating_sub(size) / 2,
        size,
        size,
    )
    .into_rgb8()
}

fn fit_pad(img: DynamicImage, size: u32) -> RgbImage {
    let rgb = img.into_rgb8();
    let (fw, fh) = rgb.dimensions();
    let mut canvas = RgbImage::new(size, size);
    image::imageops::overlay(
        &mut canvas,
        &rgb,
        (size.saturating_sub(fw) / 2) as i64,
        (size.saturating_sub(fh) / 2) as i64,
    );
    canvas
}

// ---------------------------------------------------------------------------
// Format-specific decoders
// ---------------------------------------------------------------------------

/// Decodes a RAW file with rawler.
///
/// Uses the JPEG the camera embedded in the file when its long edge is at
/// least `min_long_edge`: it is fast and its colours match the camera's own
/// rendering. Otherwise (no embedded JPEG, as in ORF, or one too small)
/// develops the RAW data. Neither result is rotated: the caller applies the
/// EXIF orientation, as for any other format.
#[cfg(feature = "raw")]
fn decode_raw(path: &Path, min_long_edge: u32) -> Result<DynamicImage, String> {
    use rawler::decoders::RawDecodeParams;
    use rawler::imgop::develop::RawDevelop;
    use rawler::rawsource::RawSource;

    let fail = |e: String| format!("failed to decode RAW {}: {e}", path.display());
    let source = RawSource::new(path).map_err(|e| fail(e.to_string()))?;
    let params = RawDecodeParams::default();

    // Some rawler code paths panic on files they do not expect. Turn a panic
    // into an error for this one photo instead of failing the whole batch.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let decoder = rawler::get_decoder(&source).map_err(|e| e.to_string())?;
        if let Ok(Some(img)) = decoder.full_image(&source, &params) {
            if img.width().max(img.height()) >= min_long_edge {
                return Ok(img);
            }
        }
        let raw = decoder
            .raw_image(&source, &params, false)
            .map_err(|e| e.to_string())?;
        let developed = RawDevelop::default()
            .develop_intermediate(&raw)
            .map_err(|e| e.to_string())?
            .to_dynamic_image()
            .ok_or_else(|| "developed image has an invalid buffer size".to_string())?;
        Ok(auto_expose(developed))
    }));
    match result {
        Ok(r) => r.map_err(fail),
        Err(_) => Err(fail("decoder panicked".into())),
    }
}

/// Brightens a developed RAW image so it looks closer to a camera JPEG.
///
/// rawler's develop applies no exposure or tone curve, so its output is dark
/// and flat next to the camera's own rendering. This scales linear light so
/// that the log-average luminance reaches a mid-grey key (Reinhard), with a
/// gain capped at 2 stops so night scenes stay dark, then rolls off the
/// highlights so nothing that was below white clips. With a gain of 1 it
/// leaves the image unchanged.
#[cfg(feature = "raw")]
fn auto_expose(img: DynamicImage) -> DynamicImage {
    const KEY: f32 = 0.18;
    const MAX_GAIN: f32 = 4.0;

    let src = img.into_rgb16();
    let to_linear: Vec<f32> = (0..=u16::MAX)
        .map(|v| {
            let c = v as f32 / u16::MAX as f32;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect();
    let to_srgb8 = |l: f32| -> u8 {
        let c = if l <= 0.003_130_8 {
            l * 12.92
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };

    // Log-average luminance on a sample of pixels.
    let (mut log_sum, mut count) = (0f64, 0u32);
    for p in src.pixels().step_by(7) {
        let [r, g, b] = p.0.map(|v| to_linear[v as usize]);
        let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        log_sum += f64::from(lum + 1e-4).ln();
        count += 1;
    }
    let log_avg = if count == 0 {
        KEY
    } else {
        (log_sum / f64::from(count)).exp() as f32
    };
    let gain = (KEY / log_avg).clamp(1.0, MAX_GAIN);

    // Extended Reinhard with the white point at `gain`: input white (1.0)
    // still maps to 1.0, and gain = 1 is the identity.
    let white_sq = gain * gain;
    let (w, h) = src.dimensions();
    let mut out = RgbImage::new(w, h);
    for (dst, p) in out.pixels_mut().zip(src.pixels()) {
        *dst = image::Rgb(p.0.map(|v| {
            let x = to_linear[v as usize] * gain;
            to_srgb8(x * (1.0 + x / white_sq) / (1.0 + x))
        }));
    }
    DynamicImage::ImageRgb8(out)
}

#[cfg(not(feature = "raw"))]
fn decode_raw(path: &Path, _min_long_edge: u32) -> Result<DynamicImage, String> {
    Err(format!(
        "RAW format not supported; rebuild with --features raw: {}",
        path.display()
    ))
}

#[cfg(feature = "heic")]
fn decode_heic(path: &Path) -> Result<DynamicImage, String> {
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

#[cfg(not(feature = "heic"))]
fn decode_heic(path: &Path) -> Result<DynamicImage, String> {
    Err(format!(
        "HEIC/HEIF format not supported; rebuild with --features heic: {}",
        path.display()
    ))
}

// ---------------------------------------------------------------------------
// Grid geometry
// ---------------------------------------------------------------------------

const GRID_COLS: u32 = 8;

fn grid_dims(n: usize) -> (u32, u32) {
    let cols = GRID_COLS;
    let rows = n.div_ceil(GRID_COLS as usize) as u32;
    (cols, rows)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    fn solid(w: u32, h: u32, pixel: Rgb<u8>) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for p in img.pixels_mut() {
            *p = pixel;
        }
        DynamicImage::ImageRgb8(img)
    }
    fn landscape() -> DynamicImage {
        solid(400, 200, Rgb([200, 100, 50]))
    }
    fn portrait() -> DynamicImage {
        solid(200, 400, Rgb([50, 100, 200]))
    }
    fn square() -> DynamicImage {
        solid(300, 300, Rgb([128, 128, 128]))
    }

    #[test]
    fn grid_dims_one() {
        assert_eq!(grid_dims(1), (8, 1));
    }
    #[test]
    fn grid_dims_two() {
        assert_eq!(grid_dims(2), (8, 1));
    }
    #[test]
    fn grid_dims_four() {
        assert_eq!(grid_dims(4), (8, 1));
    }
    #[test]
    fn grid_dims_five() {
        assert_eq!(grid_dims(5), (8, 1));
    }
    #[test]
    fn grid_dims_nine() {
        assert_eq!(grid_dims(9), (8, 2));
    }
    #[test]
    fn grid_dims_ten() {
        assert_eq!(grid_dims(10), (8, 2));
    }

    #[test]
    fn orientation_none_is_noop() {
        assert_eq!(
            apply_orientation(landscape(), None).dimensions(),
            (400, 200)
        );
    }
    #[test]
    fn orientation_1_is_noop() {
        assert_eq!(
            apply_orientation(landscape(), Some(1)).dimensions(),
            (400, 200)
        );
    }
    #[test]
    fn orientation_6_rotates_90_cw() {
        assert_eq!(
            apply_orientation(landscape(), Some(6)).dimensions(),
            (200, 400)
        );
    }
    #[test]
    fn orientation_8_rotates_90_ccw() {
        assert_eq!(
            apply_orientation(landscape(), Some(8)).dimensions(),
            (200, 400)
        );
    }
    #[test]
    fn orientation_3_rotates_180() {
        assert_eq!(
            apply_orientation(landscape(), Some(3)).dimensions(),
            (400, 200)
        );
    }
    #[test]
    fn orientation_2_flips_h() {
        assert_eq!(
            apply_orientation(landscape(), Some(2)).dimensions(),
            (400, 200)
        );
    }
    #[test]
    fn orientation_4_flips_v() {
        assert_eq!(
            apply_orientation(landscape(), Some(4)).dimensions(),
            (400, 200)
        );
    }
    #[test]
    fn orientation_5_transposes() {
        assert_eq!(
            apply_orientation(landscape(), Some(5)).dimensions(),
            (200, 400)
        );
    }
    #[test]
    fn orientation_7_transverses() {
        assert_eq!(
            apply_orientation(landscape(), Some(7)).dimensions(),
            (200, 400)
        );
    }
    #[test]
    fn orientation_unknown_is_noop() {
        assert_eq!(
            apply_orientation(landscape(), Some(9)).dimensions(),
            (400, 200)
        );
    }

    #[test]
    fn resize_short_edge_landscape() {
        let out = resize_short_edge(landscape(), 128);
        assert_eq!(out.height(), 128);
        assert!(out.width() > out.height());
    }
    #[test]
    fn resize_short_edge_portrait() {
        let out = resize_short_edge(portrait(), 128);
        assert_eq!(out.width(), 128);
        assert!(out.height() > out.width());
    }
    #[test]
    fn resize_short_edge_square() {
        assert_eq!(resize_short_edge(square(), 64).dimensions(), (64, 64));
    }

    #[test]
    fn resize_long_edge_landscape() {
        let out = resize_long_edge(landscape(), 256);
        assert_eq!(out.width(), 256);
        assert!(out.height() < out.width());
    }
    #[test]
    fn resize_long_edge_portrait() {
        let out = resize_long_edge(portrait(), 256);
        assert_eq!(out.height(), 256);
        assert!(out.width() < out.height());
    }
    #[test]
    fn resize_long_edge_already_fits_is_noop() {
        assert_eq!(resize_long_edge(landscape(), 512).dimensions(), (400, 200));
    }
    #[test]
    fn resize_long_edge_square_equal_size_is_noop() {
        assert_eq!(resize_long_edge(square(), 300).dimensions(), (300, 300));
    }

    #[test]
    fn fit_crop_output_is_square_tile() {
        assert_eq!(
            fit_crop(resize_short_edge(landscape(), 128), 128).dimensions(),
            (128, 128)
        );
    }
    #[test]
    fn fit_crop_portrait_output_is_square_tile() {
        assert_eq!(
            fit_crop(resize_short_edge(portrait(), 64), 64).dimensions(),
            (64, 64)
        );
    }

    #[test]
    fn fit_pad_output_is_square_tile() {
        assert_eq!(
            fit_pad(resize_long_edge(landscape(), 128), 128).dimensions(),
            (128, 128)
        );
    }
    #[test]
    fn fit_pad_corners_are_black() {
        let out = fit_pad(resize_long_edge(landscape(), 128), 128);
        assert_eq!(out.get_pixel(0, 0), &Rgb([0, 0, 0]));
    }
    #[test]
    fn fit_pad_center_is_not_black() {
        let out = fit_pad(
            resize_long_edge(solid(400, 200, Rgb([255, 0, 0])), 128),
            128,
        );
        assert_ne!(
            out.get_pixel(64, 64),
            &Rgb([0, 0, 0]),
            "centre should be non-black"
        );
    }

    #[test]
    fn jpeg_preview_landscape_resizes_correctly() {
        let tmpdir = std::env::temp_dir();
        let input_path = tmpdir.join("imgproc_test_in.jpg");
        let output_path = tmpdir.join("imgproc_test_out.jpg");
        // 2000×1000 landscape; after resize_long_edge(1440) → 1440×720
        solid(2000, 1000, Rgb([200, 100, 50]))
            .save(&input_path)
            .unwrap();
        let result = run_jpeg_preview(JpegPreviewInput {
            photo: PhotoInput {
                path: input_path,
                ext: ".jpg".into(),
                orientation: None,
                content_hash: "Kf3QzA2nBcR8xYvLm1P9w".into(),
            },
            max_long_edge: 1440,
            quality: 85,
            output: output_path.clone(),
        })
        .unwrap();
        assert_eq!(result.width, 1440);
        assert_eq!(result.height, 720);
        assert!(output_path.exists());
    }

    #[test]
    fn jpeg_preview_small_image_unchanged() {
        let tmpdir = std::env::temp_dir();
        let input_path = tmpdir.join("imgproc_small_in.jpg");
        let output_path = tmpdir.join("imgproc_small_out.jpg");
        // 800×600 — fits within 1440 → unchanged
        solid(800, 600, Rgb([100, 150, 200]))
            .save(&input_path)
            .unwrap();
        let result = run_jpeg_preview(JpegPreviewInput {
            photo: PhotoInput {
                path: input_path,
                ext: ".jpg".into(),
                orientation: None,
                content_hash: "Mw9xLpQrNvBsHtYjKdAcZg".into(),
            },
            max_long_edge: 1440,
            quality: 85,
            output: output_path,
        })
        .unwrap();
        assert_eq!(result.width, 800);
        assert_eq!(result.height, 600);
    }

    // Helper: write a solid-color JPEG to a temp path and return a PhotoInput.
    fn make_jpeg(name: &str, w: u32, h: u32, px: Rgb<u8>, hash: &str) -> PhotoInput {
        let path = std::env::temp_dir().join(name);
        solid(w, h, px).save(&path).unwrap();
        PhotoInput {
            path,
            ext: ".jpg".into(),
            orientation: None,
            content_hash: hash.into(),
        }
    }

    #[test]
    fn avif_grid_single_photo_produces_valid_file() {
        let output = std::env::temp_dir().join("avif_grid_1.avif");
        let result = run_avif_grid(AvifGridInput {
            photos: vec![make_jpeg(
                "ag1_a.jpg",
                300,
                300,
                Rgb([200, 100, 50]),
                "Aaaa3QzA2nBcR8xYvLm1Pw",
            )],
            tile_size: 64,
            fit: "crop".into(),
            quality: 55,
            output: output.clone(),
        })
        .unwrap();
        assert_eq!(result.rows, 1);
        assert_eq!(result.tile_size, 64);
        assert_eq!(result.photo_order, vec!["Aaaa3QzA2nBcR8xYvLm1Pw"]);
        assert!(output.exists());
        assert!(
            output.metadata().unwrap().len() > 0,
            "output file should be non-empty"
        );
    }

    #[test]
    fn avif_grid_four_photos_four_by_one() {
        let output = std::env::temp_dir().join("avif_grid_4.avif");
        let photos = vec![
            make_jpeg(
                "ag4_a.jpg",
                200,
                200,
                Rgb([255, 0, 0]),
                "Artc3QzA2nBcR8xYvLm1Pw",
            ),
            make_jpeg(
                "ag4_b.jpg",
                200,
                200,
                Rgb([0, 255, 0]),
                "Brtc3QzA2nBcR8xYvLm1Pw",
            ),
            make_jpeg(
                "ag4_c.jpg",
                200,
                200,
                Rgb([0, 0, 255]),
                "Crtc3QzA2nBcR8xYvLm1Pw",
            ),
            make_jpeg(
                "ag4_d.jpg",
                200,
                200,
                Rgb([255, 255, 0]),
                "Drtc3QzA2nBcR8xYvLm1Pw",
            ),
        ];
        let hashes: Vec<String> = photos.iter().map(|p| p.content_hash.clone()).collect();
        let result = run_avif_grid(AvifGridInput {
            photos,
            tile_size: 32,
            fit: "crop".into(),
            quality: 55,
            output: output.clone(),
        })
        .unwrap();
        assert_eq!(result.rows, 1);
        assert_eq!(result.photo_order, hashes);
        assert!(output.exists());
    }

    #[test]
    fn avif_grid_five_photos_five_by_one() {
        // 5 photos → cols=8, rows=1 (3 padding tiles)
        let output = std::env::temp_dir().join("avif_grid_5.avif");
        let photos: Vec<PhotoInput> = (0..5)
            .map(|i| {
                make_jpeg(
                    &format!("ag5_{i}.jpg"),
                    100,
                    100,
                    Rgb([i * 50, 100, 200]),
                    &format!("Photo{i:0>17}"),
                )
            })
            .collect();
        let result = run_avif_grid(AvifGridInput {
            photos,
            tile_size: 16,
            fit: "pad".into(),
            quality: 55,
            output: output.clone(),
        })
        .unwrap();
        assert_eq!(result.rows, 1);
        assert_eq!(result.photo_order.len(), 5);
        assert!(output.exists());
    }

    #[test]
    fn avif_grid_photo_order_matches_input_hashes() {
        let output = std::env::temp_dir().join("avif_grid_order.avif");
        let hashes = vec![
            "Zrt3zA2nBcR8xYvLm1P9wx",
            "Art3zA2nBcR8xYvLm1P9wx",
            "Mrt3zA2nBcR8xYvLm1P9wx",
        ];
        let photos: Vec<PhotoInput> = hashes
            .iter()
            .enumerate()
            .map(|(i, h)| make_jpeg(&format!("ago_{i}.jpg"), 80, 80, Rgb([100, 100, 100]), h))
            .collect();
        let result = run_avif_grid(AvifGridInput {
            photos,
            tile_size: 16,
            fit: "crop".into(),
            quality: 55,
            output,
        })
        .unwrap();
        // photo_order must echo input hashes in input order (caller controls ordering)
        assert_eq!(result.photo_order, hashes);
    }

    #[test]
    fn avif_grid_empty_photos_returns_error() {
        let output = std::env::temp_dir().join("avif_grid_empty.avif");
        let err = run_avif_grid(AvifGridInput {
            photos: vec![],
            tile_size: 64,
            fit: "crop".into(),
            quality: 55,
            output,
        });
        assert!(err.is_err());
    }

    // ---------------------------------------------------------------------------
    // process_line / protocol version checks
    // ---------------------------------------------------------------------------

    fn error_msg(r: Response) -> String {
        match r {
            Response::Error { error } => error,
            _ => panic!("expected Error response, got something else"),
        }
    }

    #[test]
    fn test_missing_protocol_version() {
        let line = r#"{"photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
        let err = error_msg(process_line(line, 1));
        assert!(
            err.contains("missing or invalid protocol_version"),
            "got: {err}"
        );
    }

    #[test]
    fn test_wrong_major_version() {
        let line = r#"{"protocol_version": 2, "photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
        let err = error_msg(process_line(line, 1));
        assert!(
            err.contains("unsupported protocol version 2, expected 1"),
            "got: {err}"
        );
    }

    #[test]
    fn test_version_zero_rejected() {
        let line = r#"{"protocol_version": 0, "photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
        let err = error_msg(process_line(line, 1));
        assert!(
            err.contains("unsupported protocol version 0, expected 1"),
            "got: {err}"
        );
    }

    #[test]
    fn test_protocol_version_string_rejected() {
        // protocol_version must be an integer, not a string.
        let line = r#"{"protocol_version": "1", "photo": {"path": "/tmp/x.jpg", "ext": ".jpg", "content_hash": "abc"}, "max_long_edge": 1440, "quality": 85, "output": "/tmp/out.jpg"}"#;
        let err = error_msg(process_line(line, 1));
        assert!(
            err.contains("missing or invalid protocol_version"),
            "got: {err}"
        );
    }

    #[test]
    fn test_invalid_json() {
        let err = error_msg(process_line("not json", 1));
        assert!(err.contains("invalid JSON input"), "got: {err}");
    }

    #[test]
    fn test_correct_version_dispatches_to_invalid_request() {
        // Correct protocol_version but unrecognised shape → invalid request, not version error.
        let line = r#"{"protocol_version": 1, "unknown_command": true}"#;
        let err = error_msg(process_line(line, 1));
        assert!(err.contains("invalid request"), "got: {err}");
    }

    #[cfg(feature = "heic")]
    #[test]
    fn decode_heic_returns_correct_dimensions() {
        // 64x48 gradient, generated via pillow-heif (see tests/sample-images/003.heic).
        let bytes = include_bytes!("../../tests/sample-images/003.heic");
        let tmp = std::env::temp_dir().join("image-proc-test-decode_heic.heic");
        std::fs::write(&tmp, bytes).unwrap();

        let img = decode_heic(&tmp).unwrap();

        std::fs::remove_file(&tmp).ok();
        assert_eq!(img.dimensions(), (64, 48));
    }

    // -----------------------------------------------------------------------
    // RAW decoding
    // -----------------------------------------------------------------------

    /// Sigma fp DNG, 16:9, CC0, from raw.pixls.us
    /// ("Sigma - fp - 8bit (16:9).DNG", sha256 80ae0b8f…ad0c).
    #[cfg(feature = "raw")]
    fn sample_dng() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/sample-images/004.dng")
    }

    #[cfg(feature = "raw")]
    #[test]
    fn decode_raw_dng_for_thumbnail() {
        let img = decode_photo(&sample_dng(), ".DNG", 256).unwrap();
        let (w, h) = img.dimensions();
        assert!(w >= 256, "long edge {w} smaller than requested");
        assert_eq!((w * 9 + h / 2) / h, 16, "expected 16:9, got {w}x{h}");
    }

    #[cfg(feature = "raw")]
    #[test]
    fn decode_raw_develops_when_embedded_jpeg_too_small() {
        // No embedded JPEG is this large, so this forces the develop path.
        let img = decode_raw(&sample_dng(), u32::MAX).unwrap();
        let (w, h) = img.dimensions();
        assert_eq!((w * 9 + h / 2) / h, 16, "expected 16:9, got {w}x{h}");
        // Developed output is not black: auto exposure ran.
        let mean = img.to_luma8().pixels().map(|p| p[0] as u64).sum::<u64>() / (w * h) as u64;
        assert!(mean > 40, "developed image too dark (mean luma {mean})");
    }

    #[cfg(feature = "raw")]
    #[test]
    fn decode_raw_rejects_non_raw_file_without_panicking() {
        let jpg = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/sample-images/001.jpg");
        let err = decode_photo(&jpg, ".nef", 256).unwrap_err();
        assert!(err.contains("failed to decode RAW"), "{err}");
    }

    #[cfg(feature = "raw")]
    #[test]
    fn decode_raw_missing_file_is_an_error() {
        let err = decode_photo(Path::new("/nonexistent/x.cr2"), ".cr2", 256).unwrap_err();
        assert!(err.contains("failed to decode RAW"), "{err}");
    }

    #[cfg(feature = "raw")]
    #[test]
    fn auto_expose_keeps_bright_image_unchanged() {
        // Light grey (sRGB 0.8) is above the key: gain clamps to 1 (identity).
        let v = (0.8 * u16::MAX as f32) as u16;
        let img =
            DynamicImage::ImageRgb16(image::ImageBuffer::from_pixel(8, 8, image::Rgb([v; 3])));
        let out = auto_expose(img).to_rgb8();
        let p = out.get_pixel(0, 0)[0];
        assert!((203..=205).contains(&p), "expected ~204, got {p}");
    }

    #[cfg(feature = "raw")]
    #[test]
    fn auto_expose_brightens_dark_image_and_keeps_white() {
        let dark = (0.15 * u16::MAX as f32) as u16;
        let mut buf = image::ImageBuffer::from_pixel(10, 10, image::Rgb([dark; 3]));
        buf.put_pixel(0, 0, image::Rgb([u16::MAX; 3]));
        let out = auto_expose(DynamicImage::ImageRgb16(buf)).to_rgb8();
        assert!(out.get_pixel(5, 5)[0] > 60, "dark pixels not brightened");
        assert_eq!(out.get_pixel(0, 0)[0], 255, "white must stay white");
    }
}
