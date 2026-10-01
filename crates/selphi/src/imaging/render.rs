//! Laying a photo out on the canvas: resizing to the placement, then
//! sharpening, on the background colour.

use image::RgbImage;
use image::imageops::{self, FilterType};

use super::Look;
use crate::geometry::Placement;

/// Unsharp mask after resizing: blur sigma, and the smallest difference (as
/// a fraction of full scale) that gets sharpened. The strength comes from
/// the [`Sharpening`](super::Sharpening).
const SHARPEN_SIGMA: f32 = 0.75;
const SHARPEN_THRESHOLD: f32 = 0.008 * 255.0;

/// Draws the photo onto a canvas of the look's background at `placement`:
/// resized to the placed size (which may stretch one axis for contain), then
/// sharpened as the look says. A cover picture is larger than the safe box
/// and may reach past the canvas; the canvas clips it.
pub fn render(photo: &RgbImage, placement: &Placement, look: Look) -> RgbImage {
    let resized = imageops::resize(
        photo,
        px(placement.width),
        px(placement.height),
        FilterType::Lanczos3,
    );
    let amount = look.sharpening.amount();
    let sharpened = if amount > 0.0 {
        sharpen(&resized, amount)
    } else {
        resized
    };

    let canvas = &placement.canvas;
    let mut sheet =
        RgbImage::from_pixel(px(canvas.width), px(canvas.height), look.background.rgb());
    imageops::replace(&mut sheet, &sharpened, placement.x, placement.y);
    sheet
}

/// A placement size as the `u32` the `image` crate uses.
fn px(value: i64) -> u32 {
    u32::try_from(value).expect("placement sizes are positive and fit in u32")
}

fn sharpen(image: &RgbImage, amount: f32) -> RgbImage {
    let blurred = imageops::blur(image, SHARPEN_SIGMA);
    let mut out = image.clone();
    for (value, &soft) in out.iter_mut().zip(blurred.iter()) {
        let detail = f32::from(*value) - f32::from(soft);
        if detail.abs() >= SHARPEN_THRESHOLD {
            *value = (f32::from(*value) + amount * detail)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;
    use crate::geometry::{Edge, Fit, place};
    use crate::imaging::{Background, Sharpening};

    #[test]
    fn render_puts_the_photo_exactly_at_the_placement() {
        let profile = crate::test_util::postcard();
        let photo = RgbImage::from_pixel(300, 200, Rgb([200, 0, 0]));
        let p = place(&profile, 300, 200, Fit::Contain).unwrap();
        let sheet = render(&photo, &p, Look::default());

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
    fn a_cover_picture_fills_the_safe_box_and_is_clipped_to_the_canvas() {
        let profile = crate::test_util::postcard();
        for (w, h) in [(320, 180), (180, 320), (300, 100)] {
            let photo = RgbImage::from_pixel(w, h, Rgb([200, 0, 0]));
            let p = place(&profile, w, h, Fit::Cover).unwrap();
            assert!(p.x < 0 || p.y < 0, "{w}x{h} reaches past the canvas");
            let sheet = render(&photo, &p, Look::default());

            let c = &p.canvas;
            assert_eq!(i64::from(sheet.width()), c.width, "{w}x{h}");
            assert_eq!(i64::from(sheet.height()), c.height, "{w}x{h}");
            let (left, top) = (c.trim(Edge::Left), c.trim(Edge::Top));
            let white = (top..top + c.safe_height())
                .flat_map(|y| (left..left + c.safe_width()).map(move |x| (x, y)))
                .filter(|&(x, y)| sheet.get_pixel(px(x), px(y)).0 == [255; 3])
                .count();
            assert_eq!(white, 0, "{w}x{h}: white pixels in the safe box");
        }
    }

    #[test]
    fn sharpen_leaves_flat_areas_and_faint_noise_alone() {
        let flat = RgbImage::from_pixel(8, 8, Rgb([100, 100, 100]));
        assert_eq!(sharpen(&flat, 0.75), flat);
        // A 1-level checkerboard is below the threshold.
        let faint = RgbImage::from_fn(8, 8, |x, y| Rgb([100 + ((x + y) % 2) as u8; 3]));
        assert_eq!(sharpen(&faint, 0.75), faint);
    }

    #[test]
    fn sharpen_adds_contrast_at_edges() {
        let edge = RgbImage::from_fn(8, 1, |x, _| Rgb([if x < 4 { 50 } else { 200 }; 3]));
        let out = sharpen(&edge, 0.75);
        assert!(out.get_pixel(3, 0).0[0] < 50, "dark side darkens");
        assert!(out.get_pixel(4, 0).0[0] > 200, "bright side brightens");
        assert_eq!(out.get_pixel(0, 0).0[0], 50, "far from the edge unchanged");
    }

    #[test]
    fn the_background_fills_the_canvas_around_the_picture() {
        let profile = crate::test_util::postcard();
        let photo = RgbImage::from_pixel(300, 200, Rgb([200, 0, 0]));
        let p = place(&profile, 300, 200, Fit::Contain).unwrap();
        let look = Look {
            background: Background::Black,
            ..Look::default()
        };
        let sheet = render(&photo, &p, look);
        assert_eq!(sheet.get_pixel(0, 0).0, [0, 0, 0]);
        assert_eq!(sheet.get_pixel(px(p.x), px(p.y)).0, [200, 0, 0]);
    }

    #[test]
    fn stronger_sharpening_changes_the_picture_more() {
        let profile = crate::test_util::postcard();
        // Stripes, scaled down as a camera photo is, so that there are hard
        // edges to sharpen.
        let photo = RgbImage::from_fn(3000, 2000, |x, _| {
            Rgb([if x / 5 % 2 == 0 { 50 } else { 200 }; 3])
        });
        let p = place(&profile, 3000, 2000, Fit::Contain).unwrap();
        let sheet = |sharpening| {
            let look = Look {
                sharpening,
                ..Look::default()
            };
            render(&photo, &p, look)
        };
        let off = sheet(Sharpening::Off);
        let change = |sharpening| -> u64 {
            let other = sheet(sharpening);
            off.iter()
                .zip(other.iter())
                .map(|(&a, &b)| u64::from(a.abs_diff(b)))
                .sum()
        };
        let (standard, strong) = (change(Sharpening::Standard), change(Sharpening::Strong));
        assert!(0 < standard && standard < strong, "{standard} {strong}");
    }
}
