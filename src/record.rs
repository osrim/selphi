//! The placement record: a private JPEG segment in which `prepare` notes
//! where the picture went, so that a measured print can later be turned
//! into corrected trims.
//!
//! The segment holds `selphy\0` then text: the version, the orientation, and
//! the canvas-to-picture margin on each edge in pixels, e.g.
//! `v1 portrait left=32 top=53 right=25 bottom=65`.

use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, bail};

use crate::geometry::{Edge, Orientation, Placement};

/// The APP segment number. APP15 is not used by any common format.
pub const SEGMENT: u8 = 15;
const MARKER: u8 = 0xE0 + SEGMENT;
const SIGNATURE: &[u8] = b"selphy\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub orientation: Orientation,
    /// Canvas edge to picture edge in pixels, in `Edge::ALL` order.
    margins: [i64; 4],
}

impl Record {
    pub fn of(placement: &Placement) -> Self {
        Self {
            orientation: placement.canvas.orientation,
            margins: Edge::ALL.map(|edge| placement.margin(edge)),
        }
    }

    pub fn margin_px(&self, edge: Edge) -> i64 {
        self.margins[edge as usize]
    }

    /// The segment payload, signature included.
    pub fn to_segment(&self) -> Vec<u8> {
        [SIGNATURE, self.to_string().as_bytes()].concat()
    }

    /// Reads the record from a JPEG file written by `prepare`.
    pub fn read(path: &Path) -> Result<Self> {
        let jpeg = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::find(&jpeg).with_context(|| format!("reading {}", path.display()))
    }

    /// Finds and parses the record in JPEG bytes.
    pub fn find(jpeg: &[u8]) -> Result<Self> {
        let payload = segments(jpeg)
            .into_iter()
            .find_map(|(marker, data)| {
                (marker == MARKER)
                    .then_some(data)
                    .and_then(|data| data.strip_prefix(SIGNATURE))
            })
            .context("no placement record; is this a file made by `selphy prepare`?")?;
        let text = std::str::from_utf8(payload).context("the placement record is not text")?;
        text.parse()
    }
}

impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "v1 {}", self.orientation.name())?;
        for edge in Edge::ALL {
            write!(f, " {}={}", edge.name(), self.margin_px(edge))?;
        }
        Ok(())
    }
}

impl FromStr for Record {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut words = s.split_whitespace();
        match words.next() {
            Some("v1") => {}
            other => bail!("unknown placement record version {other:?}"),
        }
        let orientation = words.next().unwrap_or_default();
        let orientation = Orientation::ALL
            .into_iter()
            .find(|o| o.name() == orientation)
            .with_context(|| format!("unknown orientation {orientation:?}"))?;

        let mut found = [None; 4];
        for word in words {
            let (name, value) = word
                .split_once('=')
                .with_context(|| format!("expected edge=pixels, got {word:?}"))?;
            let edge = Edge::ALL
                .into_iter()
                .find(|edge| edge.name() == name)
                .with_context(|| format!("unknown edge {name:?}"))?;
            let px = value
                .parse()
                .with_context(|| format!("{name} margin {value:?} is not a whole number"))?;
            found[edge as usize] = Some(px);
        }
        let mut margins = [0; 4];
        for edge in Edge::ALL {
            margins[edge as usize] = found[edge as usize]
                .with_context(|| format!("the record has no {} margin", edge.name()))?;
        }
        Ok(Self {
            orientation,
            margins,
        })
    }
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
    use crate::config::Config;
    use crate::geometry::place;
    use crate::imaging;
    use image::{Rgb, RgbImage};

    fn portrait() -> Placement {
        place(&Config::default(), 3616, 5424).unwrap()
    }

    #[test]
    fn text_form_round_trips() {
        let record = Record::of(&portrait());
        let text = record.to_string();
        assert_eq!(text, "v1 portrait left=32 top=53 right=25 bottom=65");
        assert_eq!(text.parse::<Record>().unwrap(), record);
    }

    #[test]
    fn prepared_jpeg_carries_the_record() {
        let p = portrait();
        let blank = RgbImage::from_pixel(1181, 1772, Rgb([255; 3]));
        let jpeg = imaging::encode_jpeg(&blank, &p, None).unwrap();
        assert_eq!(Record::find(&jpeg).unwrap(), Record::of(&p));
    }

    #[test]
    fn a_jpeg_without_a_record_says_so() {
        let blank = RgbImage::from_pixel(8, 8, Rgb([255; 3]));
        let jpeg = imaging::encode_plain_jpeg(&blank, 90).unwrap();
        let err = Record::find(&jpeg).unwrap_err();
        assert!(format!("{err:#}").contains("selphy prepare"), "{err:#}");
    }

    #[test]
    fn broken_files_give_errors_not_panics() {
        let p = portrait();
        let blank = RgbImage::from_pixel(1181, 1772, Rgb([255; 3]));
        let jpeg = imaging::encode_jpeg(&blank, &p, None).unwrap();
        for cut in [0, 1, 2, 3, 5, 20, 40] {
            assert!(Record::find(&jpeg[..cut]).is_err(), "cut at {cut}");
        }
        // A segment claiming a length of 0, which would underflow.
        assert!(segments(&[0xFF, 0xD8, 0xFF, 0xEF, 0, 0]).is_empty());
    }

    #[test]
    fn malformed_text_is_rejected_with_a_reason() {
        let cases = [
            ("v2 portrait left=1 top=1 right=1 bottom=1", "version"),
            ("v1 sideways left=1 top=1 right=1 bottom=1", "orientation"),
            ("v1 portrait left=1 top=1 right=1", "no bottom margin"),
            (
                "v1 portrait left=1 top=x right=1 bottom=1",
                "not a whole number",
            ),
            ("v1 portrait middle=1", "unknown edge"),
        ];
        for (text, reason) in cases {
            let err = text.parse::<Record>().unwrap_err();
            assert!(format!("{err:#}").contains(reason), "{text}: {err:#}");
        }
    }
}
