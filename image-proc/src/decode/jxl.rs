//! JPEG XL decoding through jxl-oxide.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use image::DynamicImage;
use jxl_oxide::integration::JxlDecoder;

/// Decodes a JPEG XL file. jxl-oxide applies the image's orientation, so the
/// result is upright. 16-bit and float images come back at full depth; the
/// commands reduce them to 8-bit when encoding.
pub fn decode_jxl(path: &Path) -> Result<DynamicImage, String> {
    let fail =
        |e: &dyn std::fmt::Display| format!("failed to decode JPEG XL {}: {e}", path.display());
    let file = File::open(path).map_err(|e| fail(&e))?;
    let decoder = JxlDecoder::new(BufReader::new(file)).map_err(|e| fail(&e))?;
    DynamicImage::from_decoder(decoder).map_err(|e| fail(&e))
}
