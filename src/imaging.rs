//! Pixel work: decoding a photo, converting it to sRGB, laying it out on the
//! canvas, and encoding the JPEG the SELPHY prints.

use std::path::Path;

use anyhow::{Context, Result, bail};
use image::imageops::{self, FilterType};
use image::metadata::Orientation as ExifOrientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Rgb, RgbImage};
use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};

use crate::geometry::{Edge, PPI, Placement};

/// JPEG quality for prints, the same number the bash version gave
/// ImageMagick. The two encoders' scales are similar but not identical.
const QUALITY: u8 = 88;

/// The APP segment that records the placement, and its signature. APP15 is
/// not used by any common format.
const RECORD_SEGMENT: u8 = 15;
pub const RECORD_SIGNATURE: &[u8] = b"selphy\0";

/// Unsharp mask after resizing, matching the bash version's ImageMagick
/// `-unsharp 0x0.75+0.75+0.008`: blur sigma, strength, and the smallest
/// difference (as a fraction of full scale) that gets sharpened.
const SHARPEN_SIGMA: f32 = 0.75;
const SHARPEN_AMOUNT: f32 = 0.75;
const SHARPEN_THRESHOLD: f32 = 0.008 * 255.0;

/// A decoded photo, turned the right way up, with the metadata we carry over.
pub struct Source {
    pub image: DynamicImage,
    /// The embedded colour profile, if any. `None` means sRGB.
    pub icc_profile: Option<Vec<u8>>,
    /// The raw Exif block. Its orientation tag is reset to "none", because
    /// the rotation has already been applied to the pixels.
    pub exif: Option<Vec<u8>>,
}

pub fn load(path: &Path) -> Result<Source> {
    let mut decoder = ImageReader::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .with_guessed_format()?
        .into_decoder()
        .with_context(|| format!("reading {}", path.display()))?;

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

/// Encodes the sheet as the JPEG the SELPHY accepts: baseline, 4:2:0 chroma,
/// 300 dpi. `exif` is carried over as is. The placement is recorded in a
/// private segment, so that `selphy adjust` can later turn measured white
/// borders into trims.
pub fn encode_jpeg(
    sheet: &RgbImage,
    placement: &Placement,
    exif: Option<&[u8]>,
) -> Result<Vec<u8>> {
    encode(sheet, QUALITY, |encoder| {
        if let Some(exif) = exif {
            encoder
                .add_exif_metadata(exif)
                .context("carrying over the Exif block")?;
        }
        encoder.add_app_segment(RECORD_SEGMENT, placement_record(placement))?;
        Ok(())
    })
}

/// The same JPEG format with no metadata, for sheets that are not photos.
pub fn encode_plain_jpeg(image: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    encode(image, quality, |_| Ok(()))
}

type JpegEncoder<'a> = jpeg_encoder::Encoder<&'a mut Vec<u8>>;

fn encode(
    image: &RgbImage,
    quality: u8,
    add_segments: impl FnOnce(&mut JpegEncoder) -> Result<()>,
) -> Result<Vec<u8>> {
    let width = u16::try_from(image.width()).context("image too wide for JPEG")?;
    let height = u16::try_from(image.height()).context("image too tall for JPEG")?;

    let mut bytes = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut bytes, quality);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
    encoder.set_density(jpeg_encoder::PixelDensity::dpi(PPI as u16));
    add_segments(&mut encoder)?;
    encoder.encode(image.as_raw(), width, height, jpeg_encoder::ColorType::Rgb)?;
    Ok(bytes)
}

/// `selphy\0` then text: the orientation and the canvas-to-picture margin on
/// each edge in pixels, e.g. `v1 portrait left=32 top=53 right=25 bottom=65`.
fn placement_record(placement: &Placement) -> Vec<u8> {
    let mut text = format!("v1 {}", placement.canvas.orientation.name());
    for edge in Edge::ALL {
        text.push_str(&format!(" {}={}", edge.name(), placement.margin(edge)));
    }
    [RECORD_SIGNATURE, text.as_bytes()].concat()
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
    use crate::test_util::{exif_with_orientation, fresh_dir};

    #[test]
    fn load_applies_exif_rotation_and_clears_the_tag() {
        let path = fresh_dir("imaging-rotated").join("rotated.jpg");
        let mut encoder = jpeg_encoder::Encoder::new_file(&path, 90).unwrap();
        encoder
            .add_exif_metadata(&exif_with_orientation(6)) // 6 = rotate 90 CW
            .unwrap();
        let pixels = vec![128u8; 8 * 4 * 3];
        encoder
            .encode(&pixels, 8, 4, jpeg_encoder::ColorType::Rgb)
            .unwrap();

        let source = load(&path).unwrap();
        assert_eq!((source.image.width(), source.image.height()), (4, 8));
        let exif = source.exif.expect("Exif block is kept");
        assert_eq!(
            ExifOrientation::from_exif_chunk(&exif),
            Some(ExifOrientation::NoTransforms)
        );
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
    fn render_puts_the_photo_exactly_at_the_placement() {
        let cfg = crate::config::Config::default();
        let photo = RgbImage::from_pixel(300, 200, Rgb([200, 0, 0]));
        let p = crate::geometry::place(&cfg, 300, 200).unwrap();
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

    /// The JPEG's marker segments up to the image data, as (marker, payload).
    fn segments(jpeg: &[u8]) -> Vec<(u8, &[u8])> {
        let mut out = Vec::new();
        let mut i = 2; // skip the start-of-image marker
        while i + 4 <= jpeg.len() && jpeg[i] == 0xFF {
            let marker = jpeg[i + 1];
            let len = usize::from(u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]));
            out.push((marker, &jpeg[i + 4..i + 2 + len]));
            if marker == 0xDA {
                break; // start of scan: compressed data follows
            }
            i += 2 + len;
        }
        out
    }

    fn payload<'a>(segs: &[(u8, &'a [u8])], marker: u8) -> &'a [u8] {
        segs.iter()
            .find(|(m, _)| *m == marker)
            .map(|(_, data)| *data)
            .unwrap_or_else(|| panic!("no segment 0x{marker:02X}"))
    }

    /// An encoded blank sheet for a 3:2 landscape photo, and its placement.
    fn encoded(exif: Option<&[u8]>) -> (Vec<u8>, Placement) {
        let p = crate::geometry::place(&crate::config::Config::default(), 300, 200).unwrap();
        let sheet = RgbImage::from_pixel(px(p.canvas.width), px(p.canvas.height), Rgb([255; 3]));
        (encode_jpeg(&sheet, &p, exif).unwrap(), p)
    }

    #[test]
    fn jpeg_is_baseline_420_at_300_dpi() {
        let (jpeg, p) = encoded(None);
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(i64::from(decoded.width()), p.canvas.width);

        let segs = segments(&jpeg);
        let jfif = payload(&segs, 0xE0);
        assert_eq!(&jfif[..5], b"JFIF\0");
        assert_eq!(jfif[7], 1, "density unit is dots per inch");
        assert_eq!(u16::from_be_bytes([jfif[8], jfif[9]]), 300);

        let frame = payload(&segs, 0xC0); // SOF0 = baseline
        assert_eq!(frame[5], 3, "three components");
        assert_eq!(frame[7], 0x22, "luma sampled 2x2 = 4:2:0");
        assert_eq!(frame[10], 0x11, "chroma sampled 1x1");
    }

    #[test]
    fn jpeg_records_the_placement() {
        let (jpeg, p) = encoded(None);
        let record = payload(&segments(&jpeg), 0xE0 + RECORD_SEGMENT);
        let expected = format!(
            "selphy\0v1 landscape left={} top={} right={} bottom={}",
            p.x,
            p.y,
            p.margin(Edge::Right),
            p.margin(Edge::Bottom)
        );
        assert_eq!(record, expected.as_bytes());
    }

    #[test]
    fn jpeg_carries_the_exif_block() {
        let exif = exif_with_orientation(1);
        let (jpeg, _) = encoded(Some(&exif));
        let app1 = payload(&segments(&jpeg), 0xE1);
        assert_eq!(&app1[..6], b"Exif\0\0");
        assert_eq!(&app1[6..], &exif[..]);
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
