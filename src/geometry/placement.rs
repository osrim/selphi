//! The layout algorithm: where a picture goes on its canvas so that the
//! printer's trim never reaches it.

use super::canvas::{Canvas, Edge, Orientation, px_to_mm};
use crate::config::Config;

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
}

impl Placement {
    /// Distance from the canvas edge to the picture edge.
    pub fn margin(&self, edge: Edge) -> i64 {
        match edge {
            Edge::Left => self.x,
            Edge::Top => self.y,
            Edge::Right => self.canvas.width - self.x - self.width,
            Edge::Bottom => self.canvas.height - self.y - self.height,
        }
    }

    /// White between the trim line and the picture: what should show on the
    /// card. Never negative, because the picture stays inside the safe box.
    pub fn white_mm(&self, edge: Edge) -> f64 {
        px_to_mm(self.margin(edge) - self.canvas.trim(edge))
    }
}

/// Places a `width` x `height` picture inside the safe box, uncropped.
///
/// The picture is scaled uniformly to fit, then the axis that falls short is
/// stretched by up to `max_stretch_pct`, because the card is not 2:3. It is
/// centred on the safe box, not the canvas, since the trims are asymmetric.
/// Returns `None` for an empty picture.
pub fn place(cfg: &Config, width: u32, height: u32) -> Option<Placement> {
    if width == 0 || height == 0 {
        return None;
    }
    let canvas = Canvas::new(cfg, Orientation::of(width, height));
    let (w, h) = (f64::from(width), f64::from(height));
    let (box_w, box_h) = (canvas.safe_width() as f64, canvas.safe_height() as f64);

    let scale = (box_w / w).min(box_h / h);
    let (mut fit_w, mut fit_h) = (w * scale, h * scale);

    let max_factor = 1.0 + cfg.max_stretch_pct / 100.0;
    if box_w - fit_w > box_h - fit_h {
        fit_w = (fit_w * max_factor).min(box_w);
    } else {
        fit_h = (fit_h * max_factor).min(box_h);
    }

    let (pw, ph) = (fit_w.round() as i64, fit_h.round() as i64);
    let stretch_pct = ((pw as f64 / ph as f64) / (w / h) - 1.0).abs() * 100.0;
    Some(Placement {
        x: canvas.trim(Edge::Left) + (canvas.safe_width() - pw) / 2,
        y: canvas.trim(Edge::Top) + (canvas.safe_height() - ph) / 2,
        width: pw,
        height: ph,
        stretch_pct,
        canvas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let p = place(&Config::default(), 3616, 5424).unwrap();
        assert_eq!((p.x, p.y, p.width, p.height), (32, 53, 1124, 1654));
        assert!((p.stretch_pct - 1.9).abs() < 0.05, "{}", p.stretch_pct);
        for edge in Edge::ALL {
            assert_eq!(p.white_mm(edge), 0.0, "{edge:?}");
        }
    }

    #[test]
    fn no_picture_ever_reaches_the_trim() {
        let cfg = Config::default();
        for (w, h) in SIZES {
            let p = place(&cfg, w, h).unwrap();
            for edge in Edge::ALL {
                assert!(
                    p.margin(edge) >= p.canvas.trim(edge),
                    "{w}x{h}: {edge:?} margin {} < trim {}",
                    p.margin(edge),
                    p.canvas.trim(edge)
                );
            }
            // Rounding to whole pixels can add a hair over the cap.
            assert!(p.stretch_pct <= cfg.max_stretch_pct + 0.1, "{w}x{h}");
        }
    }

    #[test]
    fn wide_pictures_keep_white_on_the_short_axis() {
        let p = place(&Config::default(), 1920, 1080).unwrap();
        assert_eq!(p.white_mm(Edge::Left), 0.0);
        assert_eq!(p.white_mm(Edge::Right), 0.0);
        assert!(p.white_mm(Edge::Top) > 7.0);
        assert!(p.white_mm(Edge::Bottom) > 7.0);
    }

    #[test]
    fn common_shapes_get_the_expected_sizes() {
        let cfg = Config::default();
        let cases = [
            ((1920, 1080), (1654, 954)),
            ((1600, 1200), (1536, 1124)),
            ((1200, 1200), (1152, 1124)),
            ((3000, 1000), (1654, 565)),
            ((1480, 1000), (1654, 1124)),
        ];
        for ((w, h), expected) in cases {
            let p = place(&cfg, w, h).unwrap();
            assert_eq!((p.width, p.height), expected, "{w}x{h}");
        }
    }

    #[test]
    fn empty_picture_has_no_placement() {
        assert_eq!(place(&Config::default(), 0, 100), None);
    }
}
