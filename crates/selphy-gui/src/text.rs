//! Text the window shows. Error sentences come from `selphy::report`.

use std::path::{Path, PathBuf};

use gpui_kit::SharedString;
use selphy::geometry::Placement;
use selphy::report::{error_chain, placement_summary, sentence};

use crate::run::Summary;

/// "3 prepared", "2 prepared, 1 failed", "Cancelled after 2 prepared", or
/// "Cancelled".
pub fn summary_text(summary: &Summary) -> String {
    let counts = match (summary.prepared, summary.failed) {
        (0, 0) => String::new(),
        (p, 0) => format!("{p} prepared"),
        (0, f) => format!("{f} failed"),
        (p, f) => format!("{p} prepared, {f} failed"),
    };
    match (summary.cancelled, counts.is_empty()) {
        (true, true) => "Cancelled".to_string(),
        (true, false) => format!("Cancelled after {counts}"),
        (false, _) => counts,
    }
}

/// "Portrait, stretched 1.9%, edge to edge": the orientation, then the
/// placement summary.
pub fn placement_text(placement: &Placement) -> String {
    let mut orientation = placement.canvas.orientation.name().to_string();
    orientation[..1].make_ascii_uppercase();
    format!("{orientation}, {}", placement_summary(placement))
}

/// An error that is not about one photo, as a sentence for a notice.
pub fn error_sentence(err: &anyhow::Error) -> SharedString {
    sentence(&error_chain(err)).into()
}

/// The path with the home folder shown as `~`.
pub fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

/// The file name of `path`, or the whole path when it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use selphy::geometry::{Fit, place};

    fn summary(prepared: usize, failed: usize, cancelled: bool) -> String {
        summary_text(&Summary {
            prepared,
            failed,
            cancelled,
        })
    }

    #[test]
    fn summary_text_leaves_out_zero_counts() {
        assert_eq!(summary(3, 0, false), "3 prepared");
        assert_eq!(summary(0, 2, false), "2 failed");
        assert_eq!(summary(2, 1, false), "2 prepared, 1 failed");
    }

    #[test]
    fn summary_text_says_when_the_run_was_cancelled() {
        assert_eq!(summary(0, 0, true), "Cancelled");
        assert_eq!(summary(2, 1, true), "Cancelled after 2 prepared, 1 failed");
    }

    #[test]
    fn placement_text_starts_with_the_orientation() {
        let profile = selphy::paper::Paper::Postcard.default_profile();
        let placement = place(&profile, 1920, 1080, Fit::Contain).unwrap();
        assert_eq!(
            placement_text(&placement),
            "Landscape, stretched 2.5%, white top 7.2, bottom 7.2 mm"
        );
    }
}
