//! Pixel work: decoding a photo, converting it to sRGB, laying it out on the
//! canvas, and encoding the JPEG the SELPHY prints.

use std::path::Path;

use anyhow::{Context, Result, bail};
use image::metadata::Orientation as ExifOrientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Rgb, RgbImage};
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};

mod jpeg;
mod render;

pub(crate) use jpeg::segments;
pub use jpeg::{AppSegment, encode_jpeg, encode_plain_jpeg};
pub use render::render;

/// A decoded photo, turned the right way up, with the metadata we carry over.
pub struct Source {
    /// The pixels, the right way up.
    pub image: DynamicImage,
    /// The embedded colour profile, if any. `None` means sRGB.
    pub icc_profile: Option<Vec<u8>>,
    /// The raw Exif block. Its orientation tag is reset to "none", because
    /// the rotation has already been applied to the pixels.
    pub exif: Option<Vec<u8>>,
}

/// Decodes the photo at `path` and applies its Exif rotation.
pub fn load(path: &Path) -> Result<Source> {
    let mut decoder = open(path)?;
    let icc_profile = decoder.icc_profile()?;
    let mut exif = decoder.exif_metadata()?;
    let orientation = exif
        .as_mut()
        .and_then(|chunk| ExifOrientation::remove_from_exif_chunk(chunk))
        .unwrap_or(ExifOrientation::NoTransforms);

    let mut image = DynamicImage::from_decoder(decoder)
        .with_context(|| format!("decoding {}", path.display()))?;
    image.apply_orientation(orientation);

    Ok(Source {
        image,
        icc_profile,
        exif,
    })
}

/// The width and height of the photo at `path` after its Exif rotation, as
/// [`load`] gives them. Reads the header and the Exif, not the pixels.
pub fn probe(path: &Path) -> Result<(u32, u32)> {
    let mut decoder = open(path)?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder
        .exif_metadata()?
        .and_then(|chunk| ExifOrientation::from_exif_chunk(&chunk))
        .unwrap_or(ExifOrientation::NoTransforms);
    let turned = matches!(
        orientation,
        ExifOrientation::Rotate90
            | ExifOrientation::Rotate270
            | ExifOrientation::Rotate90FlipH
            | ExifOrientation::Rotate270FlipH
    );
    Ok(if turned {
        (height, width)
    } else {
        (width, height)
    })
}

/// The decoder for the photo at `path`, which has read the header.
fn open(path: &Path) -> Result<impl ImageDecoder> {
    ImageReader::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .with_guessed_format()?
        .into_decoder()
        .with_context(|| format!("reading {}", path.display()))
}

/// Converts to 8-bit sRGB, which is what the SELPHY assumes. Transparency is
/// flattened onto white; an embedded colour profile is converted, not dropped.
pub fn to_srgb(image: DynamicImage, icc_profile: Option<&[u8]>) -> Result<RgbImage> {
    let is_gray = !image.color().has_color();
    let flat = flatten_on_white(image);
    let Some(bytes) = icc_profile else {
        return Ok(flat);
    };

    let profile = ColorProfile::new_from_slice(bytes).context("reading the colour profile")?;
    let src_layout = match profile.color_space {
        DataColorSpace::Rgb => Layout::Rgb,
        DataColorSpace::Gray if is_gray => Layout::Gray,
        other => bail!("unsupported colour profile: {other:?} data"),
    };
    let transform = profile.create_transform_8bit(
        src_layout,
        &ColorProfile::new_srgb(),
        Layout::Rgb,
        TransformOptions::default(),
    )?;

    let (width, height) = flat.dimensions();
    let src: Vec<u8> = match src_layout {
        Layout::Gray => flat.pixels().map(|p| p.0[0]).collect(),
        _ => flat.into_raw(),
    };
    let mut dst = vec![0u8; width as usize * height as usize * 3];
    transform.transform(&src, &mut dst)?;
    Ok(RgbImage::from_raw(width, height, dst).expect("dst is sized for width x height RGB"))
}

fn flatten_on_white(image: DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.into_rgb8();
    }
    let rgba = image.into_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let a = u32::from(a);
        let over_white = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
        Rgb([over_white(r), over_white(g), over_white(b)])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{exif_with_orientation, fresh_dir, write_jpeg};

    #[test]
    fn load_applies_exif_rotation_and_clears_the_tag() {
        let path = write_jpeg(
            &fresh_dir("imaging-rotated").join("rotated.jpg"),
            8,
            4,
            &exif_with_orientation(6), // 6 = rotate 90 CW
        );

        let source = load(&path).unwrap();
        assert_eq!((source.image.width(), source.image.height()), (4, 8));
        let exif = source.exif.expect("Exif block is kept");
        assert_eq!(
            ExifOrientation::from_exif_chunk(&exif),
            Some(ExifOrientation::NoTransforms)
        );
    }

    #[test]
    fn probe_gives_the_rotated_size_without_decoding() {
        let dir = fresh_dir("imaging-probe");
        let rotated = write_jpeg(&dir.join("rotated.jpg"), 8, 4, &exif_with_orientation(6));
        let upright = write_jpeg(&dir.join("upright.jpg"), 8, 4, &exif_with_orientation(1));
        assert_eq!(probe(&rotated).unwrap(), (4, 8));
        assert_eq!(probe(&upright).unwrap(), (8, 4));

        let broken = dir.join("broken.jpg");
        std::fs::write(&broken, b"not a jpeg").unwrap();
        let err = probe(&broken).unwrap_err();
        assert!(format!("{err:#}").contains("broken.jpg"), "{err:#}");
    }

    /// A 1x1 RGB image of one colour.
    fn pixel(rgb: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(1, 1, Rgb(rgb)))
    }

    #[test]
    fn transparency_is_flattened_onto_white() {
        let rgba = image::RgbaImage::from_fn(3, 1, |x, _| match x {
            0 => image::Rgba([0, 0, 0, 0]),     // fully transparent
            1 => image::Rgba([255, 0, 0, 255]), // opaque red
            _ => image::Rgba([0, 0, 0, 128]),   // half-transparent black
        });
        let out = to_srgb(DynamicImage::ImageRgba8(rgba), None).unwrap();
        assert_eq!(out.get_pixel(0, 0).0, [255, 255, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [255, 0, 0]);
        assert_eq!(out.get_pixel(2, 0).0, [127, 127, 127]);
    }

    #[test]
    fn no_profile_leaves_pixels_alone() {
        let out = to_srgb(pixel([200, 100, 50]), None).unwrap();
        assert_eq!(out.get_pixel(0, 0).0, [200, 100, 50]);
    }

    #[test]
    fn srgb_profile_is_close_to_a_no_op() {
        let srgb = ColorProfile::new_srgb().encode().unwrap();
        let out = to_srgb(pixel([200, 100, 50]), Some(&srgb)).unwrap();
        for (got, want) in out.get_pixel(0, 0).0.into_iter().zip([200, 100, 50]) {
            assert!(got.abs_diff(want) <= 1, "{got} vs {want}");
        }
    }

    #[test]
    fn adobe_rgb_is_converted() {
        // Adobe RGB's green primary lies outside sRGB, so the same numbers
        // mean a more saturated green, which sRGB can only clip to.
        let adobe = ColorProfile::new_adobe_rgb().encode().unwrap();
        let out = to_srgb(pixel([0, 255, 0]), Some(&adobe)).unwrap();
        let [r, g, b] = out.get_pixel(0, 0).0;
        assert!(g > 250 && r < 10 && b < 60, "{r} {g} {b}");
        let mid = to_srgb(pixel([100, 150, 200]), Some(&adobe)).unwrap();
        assert_ne!(mid.get_pixel(0, 0).0, [100, 150, 200]);
    }

    #[test]
    fn gray_profile_on_a_colour_image_is_rejected() {
        let gray = ColorProfile::new_gray_with_gamma(2.2).encode().unwrap();
        let err = to_srgb(pixel([200, 100, 50]), Some(&gray)).unwrap_err();
        assert!(format!("{err:#}").contains("Gray"), "{err:#}");
    }

    #[test]
    fn load_names_the_file_on_error() {
        let err = load(Path::new("/nonexistent/photo.jpg")).err().unwrap();
        assert!(
            format!("{err:#}").contains("/nonexistent/photo.jpg"),
            "{err:#}"
        );
    }
}
