//! The calibration sheet: keylines at known distances from each edge. After
//! printing, the lines that survive show how much the printer trims.
//!
//! Each edge carries one line per candidate trim, at 0.5mm steps, each alone
//! in its own slot with its value printed inward of it. A line d mm from the
//! edge survives exactly when the trim is less than d, so the smallest number
//! whose line still shows is the trim on that edge.

use std::fs;
use std::path::Path;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use anyhow::{Context, Result, ensure};
use image::{Rgb, RgbImage, imageops};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut, text_size};
use imageproc::rect::Rect;

use crate::config::Profile;
use crate::geometry::{Canvas, Orientation, PPI, mm_to_px};
use crate::paper::Paper;
use crate::{atomic, imaging};

/// The candidate trims, one keyline per edge each.
pub const CANDIDATES_MM: [f64; 9] = [1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5];

/// Arial, which macOS ships. Any TrueType font works.
pub const DEFAULT_FONT: &str = "/System/Library/Fonts/Supplemental/Arial.ttf";

const LINE_PX: i64 = 4;
/// Space between a line and the ends of its slot.
const SLOT_GAP_PX: i64 = 8;
const LABEL_PX: f32 = 30.0;
const NOTE_PX: f32 = 34.0;
/// Space kept free of notes at the left and right edges, for the side
/// labels.
const NOTE_MARGIN_PX: u32 = 130;
const WHITE: Rgb<u8> = Rgb([255, 255, 255]);
const BLACK: Rgb<u8> = Rgb([0, 0, 0]);
const GREY: Rgb<u8> = Rgb([0x55, 0x55, 0x55]);
const LINE_COLOURS: [Rgb<u8>; 2] = [Rgb([0xC0, 0, 0]), Rgb([0, 0x60, 0xC0])];

/// JPEG quality for the sheet: higher than for photos, so the thin lines and
/// small labels stay crisp.
const SHEET_QUALITY: u8 = 95;

/// Draws the sheet for `paper` at its `profile`'s canvas, and writes it to
/// `path` as a JPEG.
pub fn write_sheet(
    paper: Paper,
    profile: &Profile,
    orientation: Orientation,
    font: &FontVec,
    path: &Path,
) -> Result<()> {
    let jpeg =
        imaging::encode_plain_jpeg(&sheet(paper, profile, orientation, font)?, SHEET_QUALITY)?;
    atomic::write(path, jpeg)
}

/// Reads the TrueType font at `path`.
pub fn load_font(path: &Path) -> Result<FontVec> {
    let bytes = fs::read(path).with_context(|| format!("reading the font {}", path.display()))?;
    FontVec::try_from_vec(bytes).with_context(|| format!("{} is not a font", path.display()))
}

/// Draws the bracket sheet for `orientation`, at the profile's canvas size.
/// The note names `paper`. Fails if the canvas is too small to hold a slot
/// per candidate.
fn sheet(
    paper: Paper,
    profile: &Profile,
    orientation: Orientation,
    font: &FontVec,
) -> Result<RgbImage> {
    let canvas = Canvas::new(profile, orientation);
    let (w, h) = (canvas.width, canvas.height);

    // The slots share the middle 76% of each edge.
    let (h_start, h_step) = slots(w);
    let (v_start, v_step) = slots(h);
    ensure!(
        h_step.min(v_step) > 2 * SLOT_GAP_PX,
        "the {:.1}x{:.1}mm canvas is too small for the calibration sheet",
        profile.canvas_long_mm,
        profile.canvas_short_mm
    );
    let mut img = RgbImage::from_pixel(px(w), px(h), WHITE);
    for (k, &mm) in CANDIDATES_MM.iter().enumerate() {
        let d = mm_to_px(mm);
        let colour = LINE_COLOURS[k % 2];
        let k = k as i64;
        let (x0, x1) = (
            h_start + k * h_step + SLOT_GAP_PX,
            h_start + (k + 1) * h_step - SLOT_GAP_PX,
        );
        let (y0, y1) = (
            v_start + k * v_step + SLOT_GAP_PX,
            v_start + (k + 1) * v_step - SLOT_GAP_PX,
        );

        // Each line's outer side is exactly d pixels from its edge.
        draw_filled_rect_mut(&mut img, rect(x0, d, x1 - x0, LINE_PX), colour);
        draw_filled_rect_mut(
            &mut img,
            rect(x0, h - d - LINE_PX, x1 - x0, LINE_PX),
            colour,
        );
        draw_filled_rect_mut(&mut img, rect(d, y0, LINE_PX, y1 - y0), colour);
        draw_filled_rect_mut(
            &mut img,
            rect(w - d - LINE_PX, y0, LINE_PX, y1 - y0),
            colour,
        );

        // Labels sit inward of their own line, so a label never outlives it.
        let label = format!("{mm:.1}");
        let inset = LABEL_PX as i64 + 6;
        text(&mut img, font, &label, x0 + 4, d + inset);
        text(&mut img, font, &label, x0 + 4, h - d - 10);
        text_upward(&mut img, font, &label, d + inset, y1 - 4);
        text_upward(&mut img, font, &label, w - d - inset, y1 - 4);
    }

    for (i, note) in notes(paper, profile, orientation).iter().enumerate() {
        let size = note_size(font, note, w);
        centred(
            &mut img,
            font,
            note,
            size,
            (w / 2, h / 2 - 60 + 50 * i as i64),
        );
    }
    Ok(img)
}

/// The lines of text in the middle of the sheet.
fn notes(paper: Paper, profile: &Profile, orientation: Orientation) -> [String; 4] {
    [
        format!(
            "SELPHY trim bracket · {paper} · {:.1}x{:.1}mm @ {PPI}ppi · {}",
            profile.canvas_long_mm,
            profile.canvas_short_mm,
            orientation.name()
        ),
        "print Borderless, tear the tabs".to_string(),
        "per edge: find the SMALLEST number whose line still shows".to_string(),
        "that number is the trim on that edge".to_string(),
    ]
}

/// The font size for `note` on a canvas `width` px wide: [`NOTE_PX`], or
/// less when the note would reach the side labels.
fn note_size(font: &FontVec, note: &str, width: i64) -> f32 {
    let room = (width - 2 * i64::from(NOTE_MARGIN_PX)) as f32;
    let full = text_size(NOTE_PX, font, note).0 as f32;
    if full <= room {
        return NOTE_PX;
    }
    // Text width grows with the size; round down so that it fits.
    (NOTE_PX * room / full).floor()
}

/// Start and step of nine equal slots across the middle 76% of `span`.
fn slots(span: i64) -> (i64, i64) {
    let used = span as f64 * 0.76;
    let start = ((span as f64 - used) / 2.0) as i64;
    let step = (used / CANDIDATES_MM.len() as f64) as i64;
    (start, step)
}

fn px(value: i64) -> u32 {
    u32::try_from(value).expect("sheet coordinates are positive")
}

/// The rectangle with its top-left corner at (x, y). `width` and `height`
/// must be positive.
fn rect(x: i64, y: i64, width: i64, height: i64) -> Rect {
    Rect::at(x as i32, y as i32).of_size(px(width), px(height))
}

/// Label text with its baseline at `baseline`, starting at `x`.
fn text(img: &mut RgbImage, font: &FontVec, s: &str, x: i64, baseline: i64) {
    let top = baseline as f32 - font.as_scaled(PxScale::from(LABEL_PX)).ascent();
    draw_text_mut(img, BLACK, x as i32, top.round() as i32, LABEL_PX, font, s);
}

/// Label text reading bottom to top: its baseline is the vertical line
/// `x = baseline`, and it starts at `bottom` and runs upward.
fn text_upward(img: &mut RgbImage, font: &FontVec, s: &str, baseline: i64, bottom: i64) {
    let (width, height) = text_size(LABEL_PX, font, s);
    let mut flat = RgbImage::from_pixel(width, height, WHITE);
    draw_text_mut(&mut flat, BLACK, 0, 0, LABEL_PX, font, s);
    let ascent = font.as_scaled(PxScale::from(LABEL_PX)).ascent().round() as i64;
    // A quarter turn anticlockwise: the flat baseline row `ascent` becomes
    // column `ascent`, and the text's start moves to the bottom row.
    let turned = imageops::rotate270(&flat);
    darken(
        img,
        &turned,
        baseline - ascent,
        bottom - i64::from(width) + 1,
    );
}

/// Note text of `size` px, centred on `(cx, cy)`.
fn centred(img: &mut RgbImage, font: &FontVec, s: &str, size: f32, (cx, cy): (i64, i64)) {
    let (width, height) = text_size(size, font, s);
    let x = cx - i64::from(width) / 2;
    let y = cy - i64::from(height) / 2;
    draw_text_mut(img, GREY, x as i32, y as i32, size, font, s);
}

/// Copies the dark parts of `stamp` onto `img` at (x, y): each channel keeps
/// the darker value, so the stamp's white background leaves no box.
fn darken(img: &mut RgbImage, stamp: &RgbImage, x: i64, y: i64) {
    for (sx, sy, pixel) in stamp.enumerate_pixels() {
        let (tx, ty) = (x + i64::from(sx), y + i64::from(sy));
        let inside =
            (0..i64::from(img.width())).contains(&tx) && (0..i64::from(img.height())).contains(&ty);
        if inside {
            let target = img.get_pixel_mut(px(tx), px(ty));
            for (t, s) in target.0.iter_mut().zip(pixel.0) {
                *t = (*t).min(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::postcard;

    fn arial() -> FontVec {
        load_font(Path::new(DEFAULT_FONT)).expect("macOS ships Arial")
    }

    #[test]
    fn each_line_starts_exactly_its_distance_from_the_edge() {
        let profile = postcard();
        let font = arial();
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            let img = sheet(Paper::Postcard, &profile, orientation, &font).unwrap();
            let (w, h) = (i64::from(img.width()), i64::from(img.height()));
            let (h_start, h_step) = slots(w);
            let (v_start, v_step) = slots(h);
            let at = |x: i64, y: i64| *img.get_pixel(px(x), px(y));
            for (k, &mm) in CANDIDATES_MM.iter().enumerate() {
                let (d, colour, k) = (mm_to_px(mm), LINE_COLOURS[k % 2], k as i64);
                let mid_x = h_start + k * h_step + h_step / 2;
                let mid_y = v_start + k * v_step + v_step / 2;
                // (inside the line, just outside it) for top, bottom, left, right
                let probes = [
                    ((mid_x, d), (mid_x, d - 1)),
                    ((mid_x, h - d - 1), (mid_x, h - d)),
                    ((d, mid_y), (d - 1, mid_y)),
                    ((w - d - 1, mid_y), (w - d, mid_y)),
                ];
                for (inside, outside) in probes {
                    assert_eq!(at(inside.0, inside.1), colour, "{orientation:?} {mm}mm");
                    assert_eq!(at(outside.0, outside.1), WHITE, "{orientation:?} {mm}mm");
                }
            }
        }
    }

    #[test]
    fn sheet_matches_the_canvas_and_has_labels() {
        let profile = postcard();
        let img = sheet(Paper::Postcard, &profile, Orientation::Portrait, &arial()).unwrap();
        assert_eq!((img.width(), img.height()), (1181, 1772));
        let dark = img.pixels().filter(|p| p.0 == BLACK.0).count();
        assert!(dark > 1000, "labels are drawn ({dark} black pixels)");
    }

    #[test]
    fn a_canvas_too_small_for_the_slots_is_an_error() {
        let profile = Profile {
            canvas_short_mm: 10.0,
            trim_short_a_mm: 1.0,
            trim_short_b_mm: 1.0,
            ..postcard()
        };
        let err = sheet(Paper::Postcard, &profile, Orientation::Landscape, &arial()).unwrap_err();
        assert!(format!("{err:#}").contains("too small"), "{err:#}");
    }

    #[test]
    fn write_sheet_writes_a_readable_jpeg() {
        let path = crate::test_util::fresh_dir("sheet").join("calibration.jpg");
        write_sheet(
            Paper::Postcard,
            &postcard(),
            Orientation::Landscape,
            &arial(),
            &path,
        )
        .unwrap();
        let written = image::open(&path).unwrap();
        assert_eq!((written.width(), written.height()), (1772, 1181));
    }

    #[test]
    fn a_non_font_file_is_a_clear_error() {
        let path = crate::test_util::fresh_dir("font").join("not-a-font.ttf");
        fs::write(&path, b"nope").unwrap();
        let err = load_font(&path).err().unwrap();
        assert!(format!("{err:#}").contains("is not a font"), "{err:#}");
    }

    #[test]
    fn a_card_sheet_is_drawn_at_the_card_canvas() {
        let dir = crate::test_util::fresh_dir("sheet-card");
        let card = Paper::Card.starting_profile();
        for orientation in Orientation::ALL {
            let path = dir.join(format!("{}.jpg", orientation.name()));
            write_sheet(Paper::Card, &card, orientation, &arial(), &path).unwrap();
            let written = image::open(&path).unwrap();
            let canvas = Canvas::new(&card, orientation);
            assert_eq!(
                (i64::from(written.width()), i64::from(written.height())),
                (canvas.width, canvas.height)
            );
        }
    }

    #[test]
    fn the_notes_fit_between_the_side_labels_on_every_paper() {
        let font = arial();
        let longest = notes(Paper::Postcard, &postcard(), Orientation::Portrait)
            .into_iter()
            .max_by_key(|note| text_size(NOTE_PX, &font, note).0)
            .unwrap();
        for paper in Paper::ALL {
            let profile = paper.starting_profile();
            for orientation in Orientation::ALL {
                let w = Canvas::new(&profile, orientation).width;
                let room = w - 2 * i64::from(NOTE_MARGIN_PX);
                for note in notes(paper, &profile, orientation) {
                    let size = note_size(&font, &note, w);
                    let width = i64::from(text_size(size, &font, &note).0);
                    assert!(width <= room, "{paper} {orientation:?}: {note}");
                    assert!(size <= NOTE_PX);
                }
            }
        }
        // A wide canvas keeps the full size.
        assert_eq!(note_size(&font, &longest, 1772), NOTE_PX);
    }
}
