//! The canvas model: units, orientation, edges, and which measured trim
//! lands on which edge.

use crate::config::Config;

/// Output resolution. The SELPHY prints at 300 dpi.
pub const PPI: f64 = 300.0;
const MM_PER_INCH: f64 = 25.4;

/// Millimetres as whole pixels at [`PPI`], rounded to the nearest.
pub fn mm_to_px(mm: f64) -> i64 {
    (mm * PPI / MM_PER_INCH).round() as i64
}

/// Pixels at [`PPI`] as millimetres.
pub fn px_to_mm(px: i64) -> f64 {
    px as f64 * MM_PER_INCH / PPI
}

/// Which way up a picture, and so its canvas, is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Wider than tall, or square.
    Landscape,
    /// Taller than wide.
    Portrait,
}

impl Orientation {
    /// Both orientations.
    pub const ALL: [Orientation; 2] = [Orientation::Landscape, Orientation::Portrait];

    /// Square pictures count as landscape.
    pub fn of(width: u32, height: u32) -> Self {
        if height > width {
            Self::Portrait
        } else {
            Self::Landscape
        }
    }

    /// The lowercase name, as the command line and the placement record
    /// spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Landscape => "landscape",
            Self::Portrait => "portrait",
        }
    }
}

/// An edge of the canvas, as the picture is seen the right way up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The left edge.
    Left,
    /// The top edge.
    Top,
    /// The right edge.
    Right,
    /// The bottom edge.
    Bottom,
}

impl Edge {
    /// All four edges, clockwise from the left.
    pub const ALL: [Edge; 4] = [Edge::Left, Edge::Top, Edge::Right, Edge::Bottom];

    /// The lowercase name, as prompts and the placement record spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Top => "top",
            Self::Right => "right",
            Self::Bottom => "bottom",
        }
    }
}

/// One of the four trims in `Config`, named for the landscape canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trim {
    /// `trim_long_a_mm`: the landscape left end.
    LongA,
    /// `trim_long_b_mm`: the landscape right end.
    LongB,
    /// `trim_short_a_mm`: the landscape top edge.
    ShortA,
    /// `trim_short_b_mm`: the landscape bottom edge.
    ShortB,
}

impl Trim {
    /// The trim that lands on `edge` of a canvas in `orientation`. The printer
    /// rotates a portrait picture so that its top lands on long A and its left
    /// on short B, as measured on a portrait print.
    pub fn at(orientation: Orientation, edge: Edge) -> Self {
        use Edge::*;
        use Orientation::*;
        match (orientation, edge) {
            (Landscape, Left) => Self::LongA,
            (Landscape, Top) => Self::ShortA,
            (Landscape, Right) => Self::LongB,
            (Landscape, Bottom) => Self::ShortB,
            (Portrait, Left) => Self::ShortB,
            (Portrait, Top) => Self::LongA,
            (Portrait, Right) => Self::ShortA,
            (Portrait, Bottom) => Self::LongB,
        }
    }

    /// This trim's value in `cfg`.
    pub fn mm(self, cfg: &Config) -> f64 {
        match self {
            Self::LongA => cfg.trim_long_a_mm,
            Self::LongB => cfg.trim_long_b_mm,
            Self::ShortA => cfg.trim_short_a_mm,
            Self::ShortB => cfg.trim_short_b_mm,
        }
    }

    /// The config field that holds this trim, for writing.
    pub fn mm_mut(self, cfg: &mut Config) -> &mut f64 {
        match self {
            Self::LongA => &mut cfg.trim_long_a_mm,
            Self::LongB => &mut cfg.trim_long_b_mm,
            Self::ShortA => &mut cfg.trim_short_a_mm,
            Self::ShortB => &mut cfg.trim_short_b_mm,
        }
    }
}

/// The canvas handed to the printer for one orientation, in pixels. The safe
/// box is what is left after the trims: the part that reaches the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    /// Which way up the canvas is.
    pub orientation: Orientation,
    /// The whole canvas, trims included.
    pub width: i64,
    /// The whole canvas, trims included.
    pub height: i64,
    trim_left: i64,
    trim_top: i64,
    trim_right: i64,
    trim_bottom: i64,
}

impl Canvas {
    /// The configured canvas in `orientation`, with each trim on the edge it
    /// lands on.
    pub fn new(cfg: &Config, orientation: Orientation) -> Self {
        let long = mm_to_px(cfg.canvas_long_mm);
        let short = mm_to_px(cfg.canvas_short_mm);
        let (width, height) = match orientation {
            Orientation::Landscape => (long, short),
            Orientation::Portrait => (short, long),
        };
        let trim = |edge| mm_to_px(Trim::at(orientation, edge).mm(cfg));
        Self {
            orientation,
            width,
            height,
            trim_left: trim(Edge::Left),
            trim_top: trim(Edge::Top),
            trim_right: trim(Edge::Right),
            trim_bottom: trim(Edge::Bottom),
        }
    }

    /// The trim on `edge`.
    pub fn trim(&self, edge: Edge) -> i64 {
        match edge {
            Edge::Left => self.trim_left,
            Edge::Top => self.trim_top,
            Edge::Right => self.trim_right,
            Edge::Bottom => self.trim_bottom,
        }
    }

    /// The canvas side a trim on `edge` eats into: the width for the left
    /// and right edges, the height for the top and bottom.
    pub fn side_across(&self, edge: Edge) -> i64 {
        match edge {
            Edge::Left | Edge::Right => self.width,
            Edge::Top | Edge::Bottom => self.height,
        }
    }

    /// The width that reaches the card.
    pub fn safe_width(&self) -> i64 {
        self.width - self.trim_left - self.trim_right
    }

    /// The height that reaches the card.
    pub fn safe_height(&self) -> i64 {
        self.height - self.trim_top - self.trim_bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_to_px_rounds_to_the_nearest_pixel() {
        assert_eq!(mm_to_px(150.0), 1772);
        assert_eq!(mm_to_px(100.0), 1181);
        assert_eq!(mm_to_px(4.5), 53);
        assert_eq!(mm_to_px(5.5), 65);
    }

    #[test]
    fn orientation_follows_the_longer_side() {
        assert_eq!(Orientation::of(3616, 5424), Orientation::Portrait);
        assert_eq!(Orientation::of(5424, 3616), Orientation::Landscape);
        assert_eq!(Orientation::of(1200, 1200), Orientation::Landscape);
    }

    #[test]
    fn each_orientation_uses_all_four_trims_once() {
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            for trim in [Trim::LongA, Trim::LongB, Trim::ShortA, Trim::ShortB] {
                let edges = Edge::ALL
                    .into_iter()
                    .filter(|&edge| Trim::at(orientation, edge) == trim)
                    .count();
                assert_eq!(edges, 1, "{orientation:?}: {trim:?} is on {edges} edges");
            }
        }
    }

    #[test]
    fn mm_mut_writes_the_field_mm_reads() {
        let mut cfg = Config::default();
        for (i, trim) in [Trim::LongA, Trim::LongB, Trim::ShortA, Trim::ShortB]
            .into_iter()
            .enumerate()
        {
            *trim.mm_mut(&mut cfg) = 10.0 + i as f64;
            assert_eq!(trim.mm(&cfg), 10.0 + i as f64, "{trim:?}");
        }
    }

    #[test]
    fn portrait_mapping_matches_the_measured_print() {
        let cfg = Config::default();
        let trim = |edge| Trim::at(Orientation::Portrait, edge).mm(&cfg);
        // The portrait print showed 4.5mm lost at the top, 5.5mm at the bottom.
        assert_eq!(trim(Edge::Top), 4.5);
        assert_eq!(trim(Edge::Bottom), 5.5);
    }

    #[test]
    fn landscape_canvas_and_safe_box() {
        let c = Canvas::new(&Config::default(), Orientation::Landscape);
        assert_eq!((c.width, c.height), (1772, 1181));
        assert_eq!((c.safe_width(), c.safe_height()), (1654, 1124));
    }

    #[test]
    fn portrait_canvas_swaps_sides_and_remaps_trims() {
        let c = Canvas::new(&Config::default(), Orientation::Portrait);
        assert_eq!((c.width, c.height), (1181, 1772));
        let trims = Edge::ALL.map(|edge| c.trim(edge));
        // left = short B 2.7mm, top = long A 4.5mm, right = short A 2.1mm,
        // bottom = long B 5.5mm
        assert_eq!(trims, [32, 53, 25, 65]);
        assert_eq!((c.safe_width(), c.safe_height()), (1124, 1654));
    }
}
