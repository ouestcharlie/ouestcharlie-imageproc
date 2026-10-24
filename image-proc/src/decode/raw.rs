//! RAW decoding through rawler (`raw` feature).

use std::path::Path;

use image::{DynamicImage, RgbImage};

/// Decodes a RAW file with rawler.
///
/// Uses the JPEG the camera embedded in the file when its long edge is at
/// least `min_long_edge`: it is fast and its colours match the camera's own
/// rendering. Otherwise (no embedded JPEG, as in ORF, or one too small)
/// develops the RAW data. Neither result is rotated: the caller applies the
/// EXIF orientation, as for any other format.
pub fn decode_raw(path: &Path, min_long_edge: u32) -> Result<DynamicImage, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
