//! Laying a photo out on the canvas: resizing to the placement, then
//! sharpening.

use image::imageops::{self, FilterType};
use image::{Rgb, RgbImage};

use crate::geometry::Placement;

/// Unsharp mask after resizing: blur sigma, strength, and the smallest
/// difference (as a fraction of full scale) that gets sharpened.
const SHARPEN_SIGMA: f32 = 0.75;
const SHARPEN_AMOUNT: f32 = 0.75;
const SHARPEN_THRESHOLD: f32 = 0.008 * 255.0;

/// Draws the photo onto a white canvas at `placement`: resized to the placed
/// size (which may stretch one axis), then sharpened.
pub fn render(photo: &RgbImage, placement: &Placement) -> RgbImage {
    let resized = imageops::resize(
        photo,
        px(placement.width),
        px(placement.height),
        FilterType::Lanczos3,
    );
    let sharpened = sharpen(&resized);

    let canvas = &placement.canvas;
    let mut sheet = RgbImage::from_pixel(px(canvas.width), px(canvas.height), Rgb([255; 3]));
    imageops::replace(&mut sheet, &sharpened, placement.x, placement.y);
    sheet
}

/// A placement size as the `u32` the `image` crate uses.
fn px(value: i64) -> u32 {
    u32::try_from(value).expect("placement sizes are positive and fit in u32")
}

fn sharpen(image: &RgbImage) -> RgbImage {
    let blurred = imageops::blur(image, SHARPEN_SIGMA);
    let mut out = image.clone();
    for (value, &soft) in out.iter_mut().zip(blurred.iter()) {
        let detail = f32::from(*value) - f32::from(soft);
        if detail.abs() >= SHARPEN_THRESHOLD {
            *value = (f32::from(*value) + SHARPEN_AMOUNT * detail)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_puts_the_photo_exactly_at_the_placement() {
        let profile = crate::test_util::postcard();
        let photo = RgbImage::from_pixel(300, 200, Rgb([200, 0, 0]));
        let p = crate::geometry::place(&profile, 300, 200).unwrap();
        let sheet = render(&photo, &p);

        assert_eq!(i64::from(sheet.width()), p.canvas.width);
        assert_eq!(i64::from(sheet.height()), p.canvas.height);
        let at = |x: i64, y: i64| sheet.get_pixel(px(x), px(y)).0;
        let (left, top) = (p.x, p.y);
        let (right, bottom) = (p.x + p.width - 1, p.y + p.height - 1);
        assert_eq!(at(left, top), [200, 0, 0]);
        assert_eq!(at(right, bottom), [200, 0, 0]);
        assert_eq!(at(left - 1, top), [255, 255, 255]);
        assert_eq!(at(right + 1, bottom), [255, 255, 255]);
    }

    #[test]
    fn sharpen_leaves_flat_areas_and_faint_noise_alone() {
        let flat = RgbImage::from_pixel(8, 8, Rgb([100, 100, 100]));
        assert_eq!(sharpen(&flat), flat);
        // A 1-level checkerboard is below the threshold.
        let faint = RgbImage::from_fn(8, 8, |x, y| Rgb([100 + ((x + y) % 2) as u8; 3]));
        assert_eq!(sharpen(&faint), faint);
    }

    #[test]
    fn sharpen_adds_contrast_at_edges() {
        let edge = RgbImage::from_fn(8, 1, |x, _| Rgb([if x < 4 { 50 } else { 200 }; 3]));
        let out = sharpen(&edge);
        assert!(out.get_pixel(3, 0).0[0] < 50, "dark side darkens");
        assert!(out.get_pixel(4, 0).0[0] > 200, "bright side brightens");
        assert_eq!(out.get_pixel(0, 0).0[0], 50, "far from the edge unchanged");
    }
}
