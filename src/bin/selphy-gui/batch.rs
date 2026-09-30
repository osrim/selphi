//! The photos in the window and what happened to each. Plain data: the view
//! owns one `Batch` and renders it.

use std::path::{Path, PathBuf};

use selphy::prepare::Prepared;
use selphy::report::{error_chain, white_summary};

/// Stable identity of a photo in the batch, for element ids and for results
/// that arrive after the list changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhotoId(u64);

impl PhotoId {
    /// The id as an element key.
    pub fn key(self) -> u64 {
        self.0
    }
}

/// Where a photo is in the run.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Waiting,
    Preparing,
    Prepared(String),
    Failed(String),
}

impl Status {
    /// The status for a finished `prepare_one` of `source`.
    pub fn from_result(result: &anyhow::Result<Prepared>, source: &Path) -> Self {
        match result {
            Ok(prepared) => Self::Prepared(prepared_summary(prepared)),
            Err(err) => Self::Failed(failure_reason(&error_chain(err), source)),
        }
    }
}

/// The error without its leading "reading <source>: ", which the row
/// already shows as the photo's name.
fn failure_reason(chain: &str, source: &Path) -> String {
    let prefix = format!("{}: ", source.display());
    match chain.split_once(&prefix) {
        Some((_, reason)) if !reason.is_empty() => reason.to_string(),
        _ => chain.to_string(),
    }
}

/// "Portrait, stretched 1.9%, edge to edge".
fn prepared_summary(prepared: &Prepared) -> String {
    let p = &prepared.placement;
    let mut orientation = p.canvas.orientation.name().to_string();
    orientation[..1].make_ascii_uppercase();
    format!(
        "{orientation}, stretched {:.1}%, {}",
        p.stretch_pct,
        white_summary(p)
    )
}

#[derive(Debug)]
pub struct Photo {
    id: PhotoId,
    source: PathBuf,
    status: Status,
}

impl Photo {
    pub fn id(&self) -> PhotoId {
        self.id
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn status(&self) -> &Status {
        &self.status
    }
}

#[derive(Debug, Default)]
pub struct Batch {
    photos: Vec<Photo>,
    next_id: u64,
}

impl Batch {
    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    pub fn is_empty(&self) -> bool {
        self.photos.is_empty()
    }

    pub fn len(&self) -> usize {
        self.photos.len()
    }

    /// Appends the files not already in the batch, in order.
    pub fn add(&mut self, files: impl IntoIterator<Item = PathBuf>) {
        for source in files {
            if self.photos.iter().any(|photo| photo.source == source) {
                continue;
            }
            let id = PhotoId(self.next_id);
            self.next_id += 1;
            self.photos.push(Photo {
                id,
                source,
                status: Status::Waiting,
            });
        }
    }

    pub fn remove(&mut self, id: PhotoId) {
        self.photos.retain(|photo| photo.id != id);
    }

    pub fn clear(&mut self) {
        self.photos.clear();
    }

    /// Marks every photo as waiting and returns them, for a new run.
    pub fn start(&mut self) -> Vec<(PhotoId, PathBuf)> {
        self.photos
            .iter_mut()
            .map(|photo| {
                photo.status = Status::Waiting;
                (photo.id, photo.source.clone())
            })
            .collect()
    }

    /// Sets the status of `id`. A photo removed since the run started is
    /// ignored.
    pub fn set_status(&mut self, id: PhotoId, status: Status) {
        if let Some(photo) = self.photos.iter_mut().find(|photo| photo.id == id) {
            photo.status = status;
        }
    }

    /// How many photos were prepared and how many failed.
    pub fn counts(&self) -> (usize, usize) {
        let prepared = self
            .photos
            .iter()
            .filter(|photo| matches!(photo.status, Status::Prepared(_)))
            .count();
        let failed = self
            .photos
            .iter()
            .filter(|photo| matches!(photo.status, Status::Failed(_)))
            .count();
        (prepared, failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn adding_a_photo_twice_keeps_one() {
        let mut batch = Batch::default();
        batch.add(paths(&["a.jpg", "b.jpg"]));
        batch.add(paths(&["b.jpg", "c.jpg"]));
        let sources: Vec<_> = batch.photos().iter().map(Photo::source).collect();
        assert_eq!(sources, paths(&["a.jpg", "b.jpg", "c.jpg"]));
    }

    #[test]
    fn ids_stay_unique_after_removal() {
        let mut batch = Batch::default();
        batch.add(paths(&["a.jpg", "b.jpg"]));
        let first = batch.photos()[0].id();
        batch.remove(first);
        batch.add(paths(&["a.jpg"]));
        let ids: Vec<_> = batch.photos().iter().map(Photo::id).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(!ids.contains(&first));
    }

    #[test]
    fn start_resets_results_and_counts_follow_status() {
        let mut batch = Batch::default();
        batch.add(paths(&["a.jpg", "b.jpg", "c.jpg"]));
        let jobs = batch.start();
        batch.set_status(jobs[0].0, Status::Prepared("ok".into()));
        batch.set_status(jobs[1].0, Status::Failed("bad".into()));
        assert_eq!(batch.counts(), (1, 1));

        batch.start();
        assert_eq!(batch.counts(), (0, 0));
        assert!(
            batch
                .photos()
                .iter()
                .all(|photo| *photo.status() == Status::Waiting)
        );
    }

    #[test]
    fn a_result_for_a_removed_photo_is_ignored() {
        let mut batch = Batch::default();
        batch.add(paths(&["a.jpg"]));
        let jobs = batch.start();
        batch.clear();
        batch.set_status(jobs[0].0, Status::Prepared("ok".into()));
        assert!(batch.is_empty());
    }

    #[test]
    fn failure_reason_drops_the_path_the_row_shows() {
        let source = Path::new("/photos/notes.jpg");
        assert_eq!(
            failure_reason("reading /photos/notes.jpg: Format error: bad bytes", source),
            "Format error: bad bytes"
        );
        assert_eq!(
            failure_reason("the image is empty", source),
            "the image is empty"
        );
    }

    #[test]
    fn prepared_summary_starts_with_the_orientation() {
        let cfg = selphy::config::Config::default();
        let placement = selphy::geometry::place(&cfg, 1920, 1080).unwrap();
        let prepared = Prepared {
            output: PathBuf::from("out/a-selphy.jpg"),
            placement,
        };
        assert_eq!(
            prepared_summary(&prepared),
            "Landscape, stretched 2.5%, white top 7.2, bottom 7.2 mm"
        );
    }
}
