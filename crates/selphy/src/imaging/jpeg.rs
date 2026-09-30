//! The JPEG format the SELPHY prints: encoding, and walking the marker
//! segments of an encoded file.

use anyhow::{Context, Result};
use image::RgbImage;

use crate::geometry::PPI;

/// JPEG quality for prints.
const QUALITY: u8 = 88;

/// An application segment to write into a JPEG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSegment {
    /// The APP number, 1 to 15. APP0 holds the JFIF header, so the encoder
    /// refuses it.
    pub number: u8,
    /// The segment's bytes after its length field.
    pub payload: Vec<u8>,
}

/// Encodes the sheet as the JPEG the SELPHY accepts: baseline, 4:2:0 chroma,
/// 300 dpi. `exif` is carried over as is. `segments` are written after the
/// Exif, in the given order.
pub fn encode_jpeg(
    sheet: &RgbImage,
    exif: Option<&[u8]>,
    segments: &[AppSegment],
) -> Result<Vec<u8>> {
    encode(sheet, QUALITY, |encoder| {
        if let Some(exif) = exif {
            encoder
                .add_exif_metadata(exif)
                .context("carrying over the Exif block")?;
        }
        for segment in segments {
            encoder
                .add_app_segment(segment.number, segment.payload.clone())
                .with_context(|| format!("writing the APP{} segment", segment.number))?;
        }
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

/// A JPEG's marker segments up to the compressed image data, as (marker,
/// payload). Stops at the first malformed segment, so broken files give
/// fewer segments rather than a panic.
pub(crate) fn segments(jpeg: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return out;
    }
    let mut i = 2;
    while let Some(&[0xFF, marker, high, low]) = jpeg.get(i..i + 4) {
        let len = usize::from(u16::from_be_bytes([high, low]));
        let Some(payload) = len
            .checked_sub(2)
            .and_then(|body| jpeg.get(i + 4..i + 4 + body))
        else {
            break;
        };
        out.push((marker, payload));
        if marker == 0xDA {
            break; // start of scan: compressed data follows
        }
        i += 2 + len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Placement;
    use crate::test_util::exif_with_orientation;

    fn payload<'a>(segs: &[(u8, &'a [u8])], marker: u8) -> &'a [u8] {
        segs.iter()
            .find(|(m, _)| *m == marker)
            .map(|(_, data)| *data)
            .unwrap_or_else(|| panic!("no segment 0x{marker:02X}"))
    }

    /// An encoded blank sheet for a 3:2 landscape photo, and its placement.
    fn encoded(exif: Option<&[u8]>) -> (Vec<u8>, Placement) {
        let p = crate::geometry::place(&crate::test_util::postcard(), 300, 200).unwrap();
        let sheet = crate::imaging::render(&RgbImage::new(1, 1), &p);
        (encode_jpeg(&sheet, exif, &[]).unwrap(), p)
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
    fn extra_segments_follow_the_exif_in_order() {
        let exif = exif_with_orientation(1);
        let sheet = RgbImage::new(8, 8);
        let extra = [
            AppSegment {
                number: 15,
                payload: b"first".to_vec(),
            },
            AppSegment {
                number: 3,
                payload: b"second".to_vec(),
            },
        ];
        let jpeg = encode_jpeg(&sheet, Some(&exif), &extra).unwrap();
        let apps: Vec<_> = segments(&jpeg)
            .into_iter()
            .filter(|(marker, _)| (0xE1..=0xEF).contains(marker))
            .collect();
        assert_eq!(apps.len(), 3, "{apps:?}");
        assert_eq!(apps[0].0, 0xE1);
        assert_eq!(apps[1], (0xEF, &b"first"[..]));
        assert_eq!(apps[2], (0xE3, &b"second"[..]));
    }

    #[test]
    fn a_zero_segment_length_ends_the_walk() {
        // A length of 0 would underflow the payload size.
        assert!(segments(&[0xFF, 0xD8, 0xFF, 0xEF, 0, 0]).is_empty());
    }

    #[test]
    fn jpeg_carries_the_exif_block() {
        let exif = exif_with_orientation(1);
        let (jpeg, _) = encoded(Some(&exif));
        let app1 = payload(&segments(&jpeg), 0xE1);
        assert_eq!(&app1[..6], b"Exif\0\0");
        assert_eq!(&app1[6..], &exif[..]);
    }
}
