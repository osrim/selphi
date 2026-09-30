//! The layout algorithm: where a picture goes on its canvas so that the
//! printer's trim never reaches it, or so that it fills the card.

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::canvas::{Canvas, Edge, Orientation, mm_to_px, px_to_mm};
use crate::config::Profile;

/// How far a cover picture reaches past the safe box into the trim zone on
/// each edge, so that a trim that is a little off shows picture, not white.
const BLEED_MM: f64 = 1.0;

/// How a photo fills the safe box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// The whole photo, stretched on one axis by up to the max stretch,
    /// with background where its shape differs from the card's.
    Contain,
    /// The photo, unstretched, covers the card edge to edge, and the parts
    /// that do not fit are cropped.
    Cover,
}

impl Fit {
    /// Every fit.
    pub const ALL: [Fit; 2] = [Fit::Contain, Fit::Cover];

    /// The lowercase name, as the command line, the config file and the
    /// placement record spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Contain => "contain",
            Self::Cover => "cover",
        }
    }

    /// The name the user sees in the window and in `selphy config`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Contain => "Contain",
            Self::Cover => "Cover",
        }
    }
}

impl fmt::Display for Fit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Fit {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match Fit::ALL.into_iter().find(|fit| fit.name() == s) {
            Some(fit) => Ok(fit),
            None => {
                let names: Vec<&str> = Fit::ALL.map(Fit::name).into();
                bail!("unknown fit {s:?}; the fits are {}", names.join(", "))
            }
        }
    }
}

/// Where a picture goes on its canvas: its size and top-left corner, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// The canvas the picture is placed on.
    pub canvas: Canvas,
    /// The picture's left edge, from the canvas's left edge.
    pub x: i64,
    /// The picture's top edge, from the canvas's top edge.
    pub y: i64,
    /// The picture's width after scaling and stretching.
    pub width: i64,
    /// The picture's height after scaling and stretching.
    pub height: i64,
    /// How far the picture's aspect was changed, in percent.
    pub stretch_pct: f64,
    /// How the picture fills the safe box.
    pub fit: Fit,
}

impl Placement {
    /// Distance from the canvas edge to the picture edge. Negative when a
    /// cover picture reaches past the canvas edge.
    pub fn margin(&self, edge: Edge) -> i64 {
        match edge {
            Edge::Left => self.x,
            Edge::Top => self.y,
            Edge::Right => self.canvas.width - self.x - self.width,
            Edge::Bottom => self.canvas.height - self.y - self.height,
        }
    }

    /// White between the trim line and the picture: what should show on the
    /// card. Never negative: it is 0 where the picture reaches the trim line
    /// or past it.
    pub fn white_mm(&self, edge: Edge) -> f64 {
        px_to_mm((self.margin(edge) - self.canvas.trim(edge)).max(0))
    }

    /// The picture lost past the safe box on `edge`, not counting the bleed:
    /// what the fit cut off the photo. Never negative, and 0 for contain.
    pub fn cut_mm(&self, edge: Edge) -> f64 {
        let past_trim = (self.canvas.trim(edge) - self.margin(edge)).max(0);
        px_to_mm((past_trim - bleed_px(&self.canvas, edge)).max(0))
    }
}

/// The bleed on `edge` in pixels: [`BLEED_MM`], clipped to the canvas.
fn bleed_px(canvas: &Canvas, edge: Edge) -> i64 {
    mm_to_px(BLEED_MM).min(canvas.trim(edge))
}

/// Places a `width` x `height` picture on the canvas with `fit`. Returns
/// `None` for an empty picture.
///
/// Contain scales the picture uniformly to fit inside the safe box, then
/// stretches the axis that falls short by up to `max_stretch_pct`, because
/// the card is not 2:3. Cover scales it uniformly to cover the safe box plus
/// the bleed, with no stretch, and crops the axis that overflows equally on
/// both ends. Either way the picture is centred on the safe box, not the
/// canvas, since the trims are asymmetric.
pub fn place(profile: &Profile, width: u32, height: u32, fit: Fit) -> Option<Placement> {
    if width == 0 || height == 0 {
        return None;
    }
    let canvas = Canvas::new(profile, Orientation::of(width, height));
    let photo = (f64::from(width), f64::from(height));
    let max_factor = 1.0 + profile.max_stretch_pct / 100.0;
    let (pw, ph) = match fit {
        Fit::Contain => contain_size(&canvas, photo, max_factor),
        Fit::Cover => cover_size(&canvas, photo),
    };

    let (w, h) = photo;
    let stretch_pct = ((pw as f64 / ph as f64) / (w / h) - 1.0).abs() * 100.0;
    // For cover the difference is negative, and `/ 2` rounds it towards 0:
    // the right and bottom edges get the odd pixel.
    Some(Placement {
        x: canvas.trim(Edge::Left) + (canvas.safe_width() - pw) / 2,
        y: canvas.trim(Edge::Top) + (canvas.safe_height() - ph) / 2,
        width: pw,
        height: ph,
        stretch_pct,
        fit,
        canvas,
    })
}

/// The contain size of a `(w, h)` photo: inside the safe box.
fn contain_size(canvas: &Canvas, (w, h): (f64, f64), max_factor: f64) -> (i64, i64) {
    let (box_w, box_h) = (canvas.safe_width() as f64, canvas.safe_height() as f64);
    let scale = (box_w / w).min(box_h / h);
    let (mut fit_w, mut fit_h) = (w * scale, h * scale);
    if box_w - fit_w > box_h - fit_h {
        fit_w = (fit_w * max_factor).min(box_w);
    } else {
        fit_h = (fit_h * max_factor).min(box_h);
    }
    (fit_w.round() as i64, fit_h.round() as i64)
}

/// The cover size of a `(w, h)` photo: scaled uniformly over the safe box
/// plus the bleed. The picture is centred, so each axis bleeds by the larger
/// bleed of its two edges on both of them.
fn cover_size(canvas: &Canvas, (w, h): (f64, f64)) -> (i64, i64) {
    let bleed = |a, b| 2 * bleed_px(canvas, a).max(bleed_px(canvas, b));
    let need_w = (canvas.safe_width() + bleed(Edge::Left, Edge::Right)) as f64;
    let need_h = (canvas.safe_height() + bleed(Edge::Top, Edge::Bottom)) as f64;
    let scale = (need_w / w).max(need_h / h);
    (round_up(w * scale), round_up(h * scale))
}

/// Rounds up, so that a cover picture never falls a pixel short. The
/// tolerance keeps an exact size from gaining a pixel through float error.
fn round_up(px: f64) -> i64 {
    (px - 1e-6).ceil() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::postcard;

    /// Picture sizes covering the common aspects, both orientations, and
    /// extremes on either side of the card's shape.
    const SIZES: [(u32, u32); 12] = [
        (5424, 3616), // 3:2
        (3616, 5424), // 2:3
        (1920, 1080), // 16:9
        (1080, 1920), // 9:16
        (1600, 1200), // 4:3
        (1200, 1600), // 3:4
        (1200, 1200), // 1:1
        (1480, 1000), // the card's own shape
        (3000, 1000), // 3:1 panorama
        (1000, 3000), // 1:3
        (4001, 2999), // odd sizes, to exercise rounding
        (777, 1333),
    ];

    #[test]
    fn two_by_three_fills_the_safe_box() {
        let p = place(&postcard(), 3616, 5424, Fit::Contain).unwrap();
        assert_eq!((p.x, p.y, p.width, p.height), (32, 53, 1124, 1654));
        assert!((p.stretch_pct - 1.9).abs() < 0.05, "{}", p.stretch_pct);
        for edge in Edge::ALL {
            assert_eq!(p.white_mm(edge), 0.0, "{edge:?}");
        }
    }

    #[test]
    fn no_picture_ever_reaches_the_trim() {
        let profile = postcard();
        for (w, h) in SIZES {
            let p = place(&profile, w, h, Fit::Contain).unwrap();
            for edge in Edge::ALL {
                assert!(
                    p.margin(edge) >= p.canvas.trim(edge),
                    "{w}x{h}: {edge:?} margin {} < trim {}",
                    p.margin(edge),
                    p.canvas.trim(edge)
                );
            }
            // Rounding to whole pixels can add a hair over the cap.
            assert!(p.stretch_pct <= profile.max_stretch_pct + 0.1, "{w}x{h}");
        }
    }

    #[test]
    fn wide_pictures_keep_white_on_the_short_axis() {
        let p = place(&postcard(), 1920, 1080, Fit::Contain).unwrap();
        assert_eq!(p.white_mm(Edge::Left), 0.0);
        assert_eq!(p.white_mm(Edge::Right), 0.0);
        assert!(p.white_mm(Edge::Top) > 7.0);
        assert!(p.white_mm(Edge::Bottom) > 7.0);
    }

    #[test]
    fn common_shapes_get_the_expected_sizes() {
        let profile = postcard();
        let cases = [
            ((1920, 1080), (1654, 954)),
            ((1600, 1200), (1536, 1124)),
            ((1200, 1200), (1152, 1124)),
            ((3000, 1000), (1654, 565)),
            ((1480, 1000), (1654, 1124)),
        ];
        for ((w, h), expected) in cases {
            let p = place(&profile, w, h, Fit::Contain).unwrap();
            assert_eq!((p.width, p.height), expected, "{w}x{h}");
        }
    }

    #[test]
    fn empty_picture_has_no_placement() {
        for fit in Fit::ALL {
            assert_eq!(place(&postcard(), 0, 100, fit), None);
        }
    }

    #[test]
    fn cover_covers_the_safe_box_and_the_bleed() {
        let profile = postcard();
        for (w, h) in SIZES {
            let p = place(&profile, w, h, Fit::Cover).unwrap();
            assert_eq!(p.fit, Fit::Cover);
            for edge in Edge::ALL {
                let past_trim = p.canvas.trim(edge) - p.margin(edge);
                assert!(
                    past_trim >= bleed_px(&p.canvas, edge),
                    "{w}x{h}: {edge:?} reaches {past_trim} px past the trim"
                );
                assert_eq!(p.white_mm(edge), 0.0, "{w}x{h}: {edge:?}");
            }
            // Rounding each side up to whole pixels is the only change of
            // aspect.
            assert!(p.stretch_pct < 0.1, "{w}x{h}: {}", p.stretch_pct);
        }
    }

    #[test]
    fn cover_bleeds_to_the_canvas_edge_where_the_trim_is_smaller() {
        // With no trims the safe box is the canvas, so there is no bleed
        // and nothing past the canvas on the axis that fits.
        let profile = Profile {
            trim_long_a_mm: 0.0,
            trim_long_b_mm: 0.0,
            trim_short_a_mm: 0.0,
            trim_short_b_mm: 0.0,
            ..crate::test_util::postcard()
        };
        let p = place(&profile, 1920, 1080, Fit::Cover).unwrap();
        assert_eq!((p.y, p.height), (0, p.canvas.height));
        assert!(p.x < 0, "{}", p.x);
    }

    #[test]
    fn contain_cuts_nothing() {
        for (w, h) in SIZES {
            let p = place(&postcard(), w, h, Fit::Contain).unwrap();
            for edge in Edge::ALL {
                assert_eq!(p.cut_mm(edge), 0.0, "{w}x{h}: {edge:?}");
            }
        }
    }

    #[test]
    fn a_two_by_three_photo_is_not_cut_by_contain_and_cut_top_and_bottom_by_cover() {
        let contain = place(&postcard(), 3616, 5424, Fit::Contain).unwrap();
        let cover = place(&postcard(), 3616, 5424, Fit::Cover).unwrap();
        for edge in Edge::ALL {
            assert_eq!(contain.cut_mm(edge), 0.0, "{edge:?}");
        }
        // 2:3 is taller than the portrait safe box, and cover does not
        // squeeze it, so the top and bottom are cut, by the same amount.
        assert_eq!(cover.cut_mm(Edge::Left), 0.0);
        assert_eq!(cover.cut_mm(Edge::Right), 0.0);
        let (top, bottom) = (cover.cut_mm(Edge::Top), cover.cut_mm(Edge::Bottom));
        assert!(top > 1.0, "{top}");
        assert!(
            (top - bottom).abs() <= px_to_mm(1) + 1e-9,
            "{top} vs {bottom}"
        );
    }

    #[test]
    fn cover_cuts_a_wide_photo_equally_on_the_left_and_right() {
        let p = place(&postcard(), 1920, 1080, Fit::Cover).unwrap();
        let (left, right) = (p.cut_mm(Edge::Left), p.cut_mm(Edge::Right));
        assert!(left > 3.0, "{left}");
        assert!(
            (left - right).abs() <= px_to_mm(1) + 1e-9,
            "{left} vs {right}"
        );
        assert_eq!(p.cut_mm(Edge::Top), 0.0);
        assert_eq!(p.cut_mm(Edge::Bottom), 0.0);
    }

    #[test]
    fn fit_names_round_trip_through_serde_and_from_str() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Holder {
            fit: Fit,
        }
        for fit in Fit::ALL {
            let text = toml::to_string(&Holder { fit }).unwrap();
            assert_eq!(text, format!("fit = \"{}\"\n", fit.name()));
            assert_eq!(toml::from_str::<Holder>(&text).unwrap().fit, fit);
            assert_eq!(fit.name().parse::<Fit>().unwrap(), fit);
        }
        assert_eq!(Fit::ALL.map(Fit::label), ["Contain", "Cover"]);
        let err = "stretch".parse::<Fit>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "unknown fit \"stretch\"; the fits are contain, cover"
        );
    }
}
