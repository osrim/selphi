//! Text for results, shared by the command line and the window.

use std::path::Path;

use crate::geometry::{Edge, Placement};

/// "stretched 2.5%, white top 7.2, bottom 7.2 mm" for contain, "stretched
/// 2.5%, cut left 4.1, right 4.1 mm" for cover, or "stretched 1.9%, edge to
/// edge" when neither leaves 0.05 mm or more. It leaves out the orientation,
/// so that callers can put their own label first.
pub fn placement_summary(placement: &Placement) -> String {
    let white = edges_summary("white", |edge| placement.white_mm(edge));
    let cut = edges_summary("cut", |edge| placement.cut_mm(edge));
    let edges: Vec<String> = white.into_iter().chain(cut).collect();
    let edges = if edges.is_empty() {
        "edge to edge".to_string()
    } else {
        edges.join(", ")
    };
    format!("stretched {:.1}%, {edges}", placement.stretch_pct)
}

/// "white top 7.2, bottom 7.2 mm": the edges where `mm` is 0.05 or more,
/// after `what`. `None` when there are none.
fn edges_summary(what: &str, mm: impl Fn(Edge) -> f64) -> Option<String> {
    let edges: Vec<String> = Edge::ALL
        .into_iter()
        .map(|edge| (edge, mm(edge)))
        .filter(|&(_, mm)| mm >= 0.05)
        .map(|(edge, mm)| format!("{} {mm:.1}", edge.name()))
        .collect();
    (!edges.is_empty()).then(|| format!("{what} {} mm", edges.join(", ")))
}

/// The error and its causes, joined with ": ". A cause is skipped when the
/// message before it already quotes it, as the `image` crate's errors do.
pub fn error_chain(err: &anyhow::Error) -> String {
    join_causes(err.chain().map(|cause| cause.to_string()))
}

/// Why preparing `source` failed, for a line that already names `source`:
/// the error chain without the leading causes that name it, such as
/// "reading <source>". When every cause names it, the full chain.
pub fn photo_error(err: &anyhow::Error, source: &Path) -> String {
    let name = source.display().to_string();
    let reason = join_causes(
        err.chain()
            .map(|cause| cause.to_string())
            .skip_while(|text| names_path(text, &name)),
    );
    if reason.is_empty() {
        error_chain(err)
    } else {
        reason
    }
}

/// Whether `text` holds `path` as a whole word: `a.jpg` is in "reading
/// a.jpg" but not in "moving to originals/a.jpg".
fn names_path(text: &str, path: &str) -> bool {
    text.match_indices(path).any(|(start, _)| {
        let before = &text[..start];
        let after = &text[start + path.len()..];
        (before.is_empty() || before.ends_with(' '))
            && (after.is_empty() || after.starts_with([' ', ':']))
    })
}

fn join_causes(causes: impl Iterator<Item = String>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for text in causes {
        if !parts.last().is_some_and(|prev| prev.contains(&text)) {
            parts.push(text);
        }
    }
    parts.join(": ")
}

/// `text` as a sentence: first letter in capitals, final period.
pub fn sentence(text: &str) -> String {
    let mut text = text.to_string();
    if let Some(first) = text.get(..1) {
        let upper = first.to_uppercase();
        text.replace_range(..1, &upper);
    }
    if !text.ends_with('.') {
        text.push('.');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Fit, place};
    use crate::prepare::{Job, Options};
    use crate::test_util::fresh_dir;
    use crate::test_util::postcard;
    use anyhow::{Context, anyhow};
    use std::fs;

    #[test]
    fn error_chain_drops_causes_already_quoted() {
        let err = Err::<(), _>(anyhow!("bad bytes"))
            .context("Format error: bad bytes")
            .context("reading a.jpg")
            .unwrap_err();
        assert_eq!(error_chain(&err), "reading a.jpg: Format error: bad bytes");
    }

    #[test]
    fn placement_summary_gives_the_stretch_and_the_white() {
        let profile = postcard();
        let fill = place(&profile, 3616, 5424, Fit::Contain).unwrap();
        assert_eq!(placement_summary(&fill), "stretched 1.9%, edge to edge");
        let wide = place(&profile, 1920, 1080, Fit::Contain).unwrap();
        assert_eq!(
            placement_summary(&wide),
            "stretched 2.5%, white top 7.2, bottom 7.2 mm"
        );
    }

    #[test]
    fn a_cover_summary_names_the_cut_edges() {
        let profile = postcard();
        let wide = place(&profile, 1920, 1080, Fit::Cover).unwrap();
        assert_eq!(
            placement_summary(&wide),
            "stretched 2.4%, cut left 13.3, right 13.3 mm"
        );
        // The stretch cap leaves one pixel past the bleed: see
        // `geometry::placement`'s tests.
        let tall = place(&profile, 3616, 5424, Fit::Cover).unwrap();
        assert_eq!(
            placement_summary(&tall),
            "stretched 2.5%, cut top 0.1, bottom 0.1 mm"
        );
    }

    #[test]
    fn photo_error_leaves_out_the_source_it_names() {
        let dir = fresh_dir("report-photo-error");
        let bad = dir.join("broken.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let job = Job::new(
            postcard(),
            Options {
                out_dir: dir.join("out"),
                archive_dir: None,
                camera_ref: None,
                fit: Fit::Contain,
            },
        )
        .unwrap();

        let planned = crate::prepare::plan(std::slice::from_ref(&bad), &dir.join("out")).unwrap();
        let err = job.run_one(&planned[0]).unwrap_err();
        let text = photo_error(&err, &bad);
        assert!(!text.contains(&bad.display().to_string()), "{text}");
        assert!(!text.contains("broken.jpg"), "{text}");
        assert!(!text.is_empty());
    }

    #[test]
    fn photo_error_keeps_an_error_that_does_not_name_the_source() {
        let err = Err::<(), _>(anyhow!("bad bytes"))
            .context("the image is empty")
            .unwrap_err();
        assert_eq!(
            photo_error(&err, Path::new("/photos/a.jpg")),
            "the image is empty: bad bytes"
        );
    }

    #[test]
    fn photo_error_keeps_the_chain_when_every_cause_names_the_source() {
        let err = anyhow!("opening /photos/a.jpg");
        assert_eq!(
            photo_error(&err, Path::new("/photos/a.jpg")),
            "opening /photos/a.jpg"
        );
    }

    #[test]
    fn photo_error_keeps_a_cause_that_names_another_path() {
        let err = Err::<(), _>(anyhow!("permission denied"))
            .context("moving to originals/a.jpg")
            .unwrap_err();
        assert_eq!(
            photo_error(&err, Path::new("a.jpg")),
            "moving to originals/a.jpg: permission denied"
        );
    }

    #[test]
    fn sentence_capitalises_and_ends_with_a_period() {
        assert_eq!(
            sentence("listing trip: permission denied"),
            "Listing trip: permission denied."
        );
        assert_eq!(sentence("Done."), "Done.");
    }
}
