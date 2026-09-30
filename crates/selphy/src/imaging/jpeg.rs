//! The JPEG format the SELPHY prints, with the placement record in a private
//! segment.

use anyhow::{Context, Result};
use image::RgbImage;

use crate::geometry::{PPI, Placement};
use crate::record::{self, Record};

/// JPEG quality for prints.
const QUALITY: u8 = 88;

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
        encoder.add_app_segment(record::SEGMENT, Record::of(placement).to_segment())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::segments;
    use crate::test_util::exif_with_orientation;

    fn payload<'a>(segs: &[(u8, &'a [u8])], marker: u8) -> &'a [u8] {
        segs.iter()
            .find(|(m, _)| *m == marker)
            .map(|(_, data)| *data)
            .unwrap_or_else(|| panic!("no segment 0x{marker:02X}"))
    }

    /// An encoded blank sheet for a 3:2 landscape photo, and its placement.
    fn encoded(exif: Option<&[u8]>) -> (Vec<u8>, Placement) {
        let p = crate::geometry::place(&crate::config::Config::default(), 300, 200).unwrap();
        let sheet = crate::imaging::render(&RgbImage::new(1, 1), &p);
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
    fn jpeg_carries_the_exif_block() {
        let exif = exif_with_orientation(1);
        let (jpeg, _) = encoded(Some(&exif));
        let app1 = payload(&segments(&jpeg), 0xE1);
        assert_eq!(&app1[..6], b"Exif\0\0");
        assert_eq!(&app1[6..], &exif[..]);
    }
}
