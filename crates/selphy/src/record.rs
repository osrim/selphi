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

use crate::geometry::{Edge, Orientation, Placement, px_to_mm};
use crate::imaging::{self, AppSegment};

/// The APP segment number. APP15 is not used by any common format.
const SEGMENT: u8 = 15;
const MARKER: u8 = 0xE0 + SEGMENT;
const SIGNATURE: &[u8] = b"selphy\0";

/// Where `prepare` put the picture: enough to turn the white measured on a
/// print back into trims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The canvas's orientation.
    pub orientation: Orientation,
    /// Canvas edge to picture edge in pixels, in `Edge::ALL` order.
    margins: [i64; 4],
}

impl Record {
    /// The record of `placement`.
    pub fn of(placement: &Placement) -> Self {
        Self {
            orientation: placement.canvas.orientation,
            margins: Edge::ALL.map(|edge| placement.margin(edge)),
        }
    }

    /// Canvas edge to picture edge on `edge`, in pixels.
    pub fn margin_px(&self, edge: Edge) -> i64 {
        self.margins[edge as usize]
    }

    /// The trim on `edge` when the card shows `white_mm` of white there:
    /// margin - white, not rounded. [`Placement::white_mm`] is the forward
    /// rule, white = margin - trim.
    pub fn trim_mm(&self, edge: Edge, white_mm: f64) -> f64 {
        px_to_mm(self.margin_px(edge)) - white_mm
    }

    /// The JPEG segment that holds this record.
    pub fn segment(&self) -> AppSegment {
        AppSegment {
            number: SEGMENT,
            payload: [SIGNATURE, self.to_string().as_bytes()].concat(),
        }
    }

    /// Reads the record from a JPEG file written by `prepare`.
    pub fn read(path: &Path) -> Result<Self> {
        let jpeg = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::find(&jpeg).with_context(|| format!("reading {}", path.display()))
    }

    /// Finds and parses the record in JPEG bytes.
    pub fn find(jpeg: &[u8]) -> Result<Self> {
        let payload = imaging::segments(jpeg)
            .into_iter()
            .find_map(|(marker, data)| {
                (marker == MARKER)
                    .then_some(data)
                    .and_then(|data| data.strip_prefix(SIGNATURE))
            })
            .context("no placement record; only files made by `selphy prepare` have one")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::geometry::{Trim, place};
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
        let record = Record::of(&p);
        let jpeg = imaging::encode_jpeg(&blank, None, &[record.segment()]).unwrap();
        assert_eq!(Record::find(&jpeg).unwrap(), record);
    }

    /// The measured white turned back into a trim gives the configured trim:
    /// `trim_mm` undoes `Placement::white_mm`.
    #[test]
    fn the_expected_white_gives_back_the_configured_trims() {
        let cfg = Config {
            trim_long_a_mm: 5.5,
            trim_long_b_mm: 4.25,
            trim_short_a_mm: 3.64,
            trim_short_b_mm: 2.73,
            ..Config::default()
        };
        let half_px_mm = px_to_mm(1) / 2.0;
        let sizes = [
            (5424, 3616),
            (3616, 5424),
            (1920, 1080),
            (1080, 1920),
            (1200, 1200),
            (3000, 1000),
            (777, 1333),
        ];
        let mut orientations = Vec::new();
        for (w, h) in sizes {
            let p = place(&cfg, w, h).unwrap();
            let record = Record::of(&p);
            orientations.push(record.orientation);
            for edge in Edge::ALL {
                let trim_mm = record.trim_mm(edge, p.white_mm(edge));
                let configured = Trim::at(record.orientation, edge).mm(&cfg);
                assert!(
                    (trim_mm - configured).abs() <= half_px_mm,
                    "{w}x{h} {edge:?}: {trim_mm} vs {configured}"
                );
            }
        }
        for orientation in Orientation::ALL {
            assert!(orientations.contains(&orientation), "{orientation:?}");
        }
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
        let jpeg = imaging::encode_jpeg(&blank, None, &[Record::of(&p).segment()]).unwrap();
        for cut in [0, 1, 2, 3, 5, 20, 40] {
            assert!(Record::find(&jpeg[..cut]).is_err(), "cut at {cut}");
        }
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
