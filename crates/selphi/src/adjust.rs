//! Corrected trims from a measured print. `prepare` records how far the
//! picture sat from each canvas edge; the card shows how much of that the
//! printer actually trimmed. The difference is the true trim.

use anyhow::{Result, bail};

use crate::config::Profile;
use crate::geometry::{Canvas, Edge, Fit, px_to_mm};
use crate::record::Record;

/// The profile after measuring a print of the file that `record` came from.
/// Each measurement is the white on that edge of the card in mm, or the
/// picture lost there as a negative number. Edges not in `measured` keep
/// their trim.
///
/// [`Record::trim_mm`] turns each measurement into a trim: margin - white.
/// The margin is canvas edge to picture edge, so this holds on edges with
/// deliberate white too.
///
/// `profile` must be the profile the file was prepared with. A record that
/// [`check_record`] refuses is an error. A measurement that gives a negative
/// trim, or a trim over half the canvas, is an error: it is a wrong reading.
/// A result that leaves nothing to print is an error from
/// [`Profile::with_trims`].
pub fn apply_measurements(
    profile: &Profile,
    record: &Record,
    measured: &[(Edge, f64)],
) -> Result<Profile> {
    check_record(profile, record)?;
    let canvas = Canvas::new(profile, record.orientation);
    let mut trims = Vec::with_capacity(measured.len());
    for &(edge, white_mm) in measured {
        let trim_mm = round_to_hundredths(record.trim_mm(edge, white_mm));
        if trim_mm < 0.0 {
            let margin_mm = px_to_mm(record.margin_px(edge));
            bail!(
                "{white_mm} mm of white on the {} edge is more than the {margin_mm:.2} mm \
                 margin the picture had there; check the measurement",
                edge.name()
            );
        }
        let limit_mm = px_to_mm(canvas.side_across(edge)) / 2.0;
        if trim_mm > limit_mm {
            bail!(
                "the {} edge would get a trim of {trim_mm:.2} mm, over half the canvas \
                 ({limit_mm:.2} mm); check the measurement and its sign",
                edge.name()
            );
        }
        trims.push((edge, trim_mm));
    }
    profile.with_trims(record.orientation, &trims)
}

/// Checks that a print of the file that `record` came from can correct
/// `profile`: the file must be a contain print, because a cover print's
/// margins do not show the trim, and the profile's canvas must be the canvas
/// the file was prepared on. Call it before asking for measurements, so that
/// the user does not measure a print that cannot be used.
pub fn check_record(profile: &Profile, record: &Record) -> Result<()> {
    if record.fit == Fit::Cover {
        bail!(
            "a {} print cannot be measured; prepare it with the {} fit (--fit {})",
            Fit::Cover.label(),
            Fit::Contain.label(),
            Fit::Contain.name()
        );
    }
    let canvas = Canvas::new(profile, record.orientation);
    if (canvas.width, canvas.height) != record.canvas_px {
        bail!(
            "the {} canvas has changed since this file was prepared; prepare and print it again",
            record.paper
        );
    }
    Ok(())
}

/// Keeps the TOML tidy: 2.7253 is saved as 2.73.
fn round_to_hundredths(mm: f64) -> f64 {
    (mm * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::postcard;

    /// The profile the user's portrait 3:2 print was made with.
    fn old_profile() -> Profile {
        Profile {
            trim_long_a_mm: 5.5,
            trim_long_b_mm: 5.5,
            trim_short_a_mm: 3.64,
            trim_short_b_mm: 3.73,
            ..postcard()
        }
    }

    /// Margins of 3.73, 5.50, 3.64 and 5.50 mm, on the postcard canvas.
    fn portrait_record() -> Record {
        "v2 postcard contain portrait canvas=1181x1772 left=44 top=65 right=43 bottom=65"
            .parse()
            .unwrap()
    }

    #[test]
    fn the_measured_portrait_print_gives_the_expected_trims() {
        let measured = [
            (Edge::Top, 1.0),
            (Edge::Left, 1.0),
            (Edge::Right, 1.5),
            (Edge::Bottom, 0.0),
        ];
        let after = apply_measurements(&old_profile(), &portrait_record(), &measured).unwrap();
        // portrait top = long A, bottom = long B, left = short B, right = short A
        assert_eq!(after.trim_long_a_mm, 4.5);
        assert_eq!(after.trim_short_b_mm, 2.73);
        assert_eq!(after.trim_short_a_mm, 2.14);
        assert_eq!(after.trim_long_b_mm, 5.5);
        assert_eq!(after.canvas_long_mm, old_profile().canvas_long_mm);
    }

    #[test]
    fn unmeasured_edges_keep_their_trim() {
        let before = old_profile();
        let after = apply_measurements(&before, &portrait_record(), &[(Edge::Top, 1.0)]).unwrap();
        assert_eq!(after.trim_long_a_mm, 4.5);
        assert_eq!(
            Profile {
                trim_long_a_mm: before.trim_long_a_mm,
                ..after
            },
            before
        );
    }

    #[test]
    fn a_cut_picture_raises_the_trim() {
        let after = apply_measurements(&old_profile(), &portrait_record(), &[(Edge::Bottom, -0.5)])
            .unwrap();
        assert_eq!(after.trim_long_b_mm, 6.0);
    }

    #[test]
    fn more_white_than_margin_is_an_error_naming_the_edge() {
        let err = apply_measurements(&old_profile(), &portrait_record(), &[(Edge::Left, 4.0)])
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("left edge"), "{message}");
        assert!(message.contains("3.73 mm margin"), "{message}");
    }

    #[test]
    fn a_trim_over_half_the_canvas_is_an_error_naming_the_edge() {
        // Portrait canvas: 100mm wide, 150mm high.
        let err = apply_measurements(&old_profile(), &portrait_record(), &[(Edge::Right, -50.0)])
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("right edge"), "{message}");
        assert!(
            message.contains("over half the canvas (50.00 mm)"),
            "{message}"
        );
        // The same cut fits on the taller side.
        assert!(
            apply_measurements(&old_profile(), &portrait_record(), &[(Edge::Top, -50.0)]).is_ok()
        );
    }

    #[test]
    fn trims_that_leave_nothing_are_an_error_naming_both_edges() {
        // Portrait canvas: 150mm high. 75mm trims are each within half of it,
        // but together they leave 0 px.
        let measured = [(Edge::Top, -69.5), (Edge::Bottom, -69.5)];
        let err = apply_measurements(&old_profile(), &portrait_record(), &measured).unwrap_err();
        let message = format!("{err:#}");
        assert!(
            message.contains("the top and bottom trims (150 mm) leave nothing of the 150 mm side"),
            "{message}"
        );
    }

    #[test]
    fn a_cover_print_is_refused() {
        let cover: Record =
            "v2 postcard cover portrait canvas=1181x1772 left=-10 top=-30 right=-10 bottom=-30"
                .parse()
                .unwrap();
        let err = check_record(&old_profile(), &cover).unwrap_err();
        assert_eq!(
            err.to_string(),
            "a Cover print cannot be measured; prepare it with the Contain fit (--fit \
             contain)"
        );
        assert!(apply_measurements(&old_profile(), &cover, &[(Edge::Top, 1.0)]).is_err());
    }

    #[test]
    fn a_changed_canvas_is_refused() {
        let smaller = Profile {
            canvas_long_mm: 148.0,
            ..old_profile()
        };
        let err =
            apply_measurements(&smaller, &portrait_record(), &[(Edge::Top, 1.0)]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the postcard canvas has changed since this file was prepared; prepare and print it \
             again"
        );
        assert!(check_record(&smaller, &portrait_record()).is_err());
        assert!(check_record(&old_profile(), &portrait_record()).is_ok());
    }
}
