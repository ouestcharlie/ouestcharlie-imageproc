//! `avif_grid` command: many photos → one AVIF grid of square tiles.

use std::path::Path;

use image::RgbImage;
use rayon::prelude::*;
use rgb::FromSlice;

use crate::decode::decode_oriented;
use crate::protocol::{AvifGridInput, AvifGridOutput};
use crate::transform::{fit_crop, fit_pad, resize_long_edge, resize_short_edge};

const GRID_COLS: u32 = 8;

pub fn run_avif_grid(input: AvifGridInput) -> Result<AvifGridOutput, Box<dyn std::error::Error>> {
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

fn decode_and_prepare(
    path: &Path,
    ext: &str,
    orientation: Option<u8>,
    tile_size: u32,
    fit: &str,
) -> Result<RgbImage, String> {
    let img = decode_oriented(path, ext, orientation, tile_size)?;
    Ok(if fit == "crop" {
        fit_crop(resize_short_edge(img, tile_size), tile_size)
    } else {
        fit_pad(resize_long_edge(img, tile_size), tile_size)
    })
}

/// Photos are always arranged in 8-column rows; the last row is padded.
fn grid_dims(n: usize) -> (u32, u32) {
    let cols = GRID_COLS;
    let rows = n.div_ceil(GRID_COLS as usize) as u32;
    (cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PhotoInput;
    use crate::transform::test_util::solid;
    use image::Rgb;

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
}
