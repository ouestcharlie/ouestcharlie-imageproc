//! Pixel transforms shared by the commands: EXIF orientation, resize, fit.

use image::{DynamicImage, GenericImageView, RgbImage};

pub fn apply_orientation(img: DynamicImage, orientation: Option<u8>) -> DynamicImage {
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

pub fn resize_short_edge(img: DynamicImage, size: u32) -> DynamicImage {
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

pub fn resize_long_edge(img: DynamicImage, size: u32) -> DynamicImage {
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

pub fn fit_crop(img: DynamicImage, size: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    img.crop_imm(
        w.saturating_sub(size) / 2,
        h.saturating_sub(size) / 2,
        size,
        size,
    )
    .into_rgb8()
}

pub fn fit_pad(img: DynamicImage, size: u32) -> RgbImage {
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

#[cfg(test)]
pub(crate) mod test_util {
    use image::{DynamicImage, Rgb, RgbImage};

    pub fn solid(w: u32, h: u32, pixel: Rgb<u8>) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for p in img.pixels_mut() {
            *p = pixel;
        }
        DynamicImage::ImageRgb8(img)
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::solid;
    use super::*;
    use image::Rgb;

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
}
