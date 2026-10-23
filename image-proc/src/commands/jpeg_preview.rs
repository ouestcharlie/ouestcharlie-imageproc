//! `jpeg_preview` command: one photo → resized JPEG.

use image::GenericImageView;

use crate::decode::decode_photo;
use crate::protocol::{JpegPreviewInput, JpegPreviewOutput};
use crate::transform::{apply_orientation, resize_long_edge};

pub fn run_jpeg_preview(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PhotoInput;
    use crate::transform::test_util::solid;
    use image::Rgb;

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
}
