//! The photos in the window, the selected one, what happened to each, and
//! the undo history of the list. Plain data: the main window owns one
//! `Batch` and renders it.

use std::path::{Path, PathBuf};

use gpui_kit::base::History;
use selphi::prepare::Done;
use selphi::report::photo_error;

/// How many list changes Undo can take back.
const UNDO_DEPTH: usize = 50;

/// Stable identity of a photo in the batch, for element ids, caches, and
/// results that arrive after the list changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    Prepared { output: PathBuf },
    Failed(String),
}

impl Status {
    /// The status for a finished `Job::run_one` of `source`: the output, or
    /// why it failed, without the path the row already shows.
    pub fn from_result(result: &anyhow::Result<Done>, source: &Path) -> Self {
        match result {
            Ok(done) => Self::Prepared {
                output: done.prepared.output.clone(),
            },
            Err(err) => Self::Failed(photo_error(err, source)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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

    /// The output once the photo has been prepared, else the source: what
    /// Show in Finder shows.
    pub fn shown_file(&self) -> &Path {
        match &self.status {
            Status::Prepared { output, .. } => output,
            _ => &self.source,
        }
    }
}

/// The photo list. While a run is in progress the list is locked: Add,
/// Remove, Clear, Undo and Redo do nothing, and only statuses change.
#[derive(Debug)]
pub struct Batch {
    photos: Vec<Photo>,
    next_id: u64,
    selected: Option<PhotoId>,
    running: bool,
    /// The list after each change, oldest first. The current entry is kept
    /// up to date with the statuses before each change, so that Undo brings
    /// back the results too.
    history: History<Vec<Photo>>,
}

impl Default for Batch {
    fn default() -> Self {
        let mut history = History::new().max_entries(UNDO_DEPTH + 1);
        history.push(Vec::new());
        Self {
            photos: Vec::new(),
            next_id: 0,
            selected: None,
            running: false,
            history,
        }
    }
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

    pub fn photo(&self, id: PhotoId) -> Option<&Photo> {
        self.photos.iter().find(|photo| photo.id == id)
    }

    /// Appends the files not already in the batch, in order, and returns how
    /// many were added. Selects the first one added when nothing is
    /// selected.
    pub fn add(&mut self, files: impl IntoIterator<Item = PathBuf>) -> usize {
        if self.running {
            return 0;
        }
        let before = self.photos.len();
        let mut added = self.photos.clone();
        for source in files {
            if added.iter().any(|photo| photo.source == source) {
                continue;
            }
            let id = PhotoId(self.next_id);
            self.next_id += 1;
            added.push(Photo {
                id,
                source,
                status: Status::Waiting,
            });
        }
        let count = added.len() - before;
        if count > 0 {
            let first = added[before].id;
            self.commit(added);
            self.selected = self.selected.or(Some(first));
        }
        count
    }

    /// Removes the photo. When it was selected, the next row is selected,
    /// or the previous one when it was the last.
    pub fn remove(&mut self, id: PhotoId) {
        if self.running {
            return;
        }
        let Some(ix) = self.index_of(id) else {
            return;
        };
        let mut rest = self.photos.clone();
        rest.remove(ix);
        if self.selected == Some(id) {
            self.selected = rest.get(ix).or(rest.last()).map(Photo::id);
        }
        self.commit(rest);
    }

    pub fn remove_selected(&mut self) {
        if let Some(id) = self.selected {
            self.remove(id);
        }
    }

    pub fn clear(&mut self) {
        if self.running || self.photos.is_empty() {
            return;
        }
        self.commit(Vec::new());
        self.selected = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.running && self.history.can_back()
    }

    pub fn can_redo(&self) -> bool {
        !self.running && self.history.can_forward()
    }

    /// Takes back the last Add, Remove or Clear.
    pub fn undo(&mut self) -> bool {
        if !self.can_undo() {
            return false;
        }
        self.history.replace_current(self.photos.clone());
        match self.history.back() {
            Some(photos) => self.restore(photos),
            None => false,
        }
    }

    /// Makes the last undone change again.
    pub fn redo(&mut self) -> bool {
        if !self.can_redo() {
            return false;
        }
        self.history.replace_current(self.photos.clone());
        match self.history.forward() {
            Some(photos) => self.restore(photos),
            None => false,
        }
    }

    pub fn selected(&self) -> Option<&Photo> {
        self.selected.and_then(|id| self.photo(id))
    }

    pub fn selected_id(&self) -> Option<PhotoId> {
        self.selected
    }

    /// Selects the photo, if it is in the batch.
    pub fn select(&mut self, id: PhotoId) {
        if self.index_of(id).is_some() {
            self.selected = Some(id);
        }
    }

    /// Moves the selection one row down, or up for a negative `step`. It
    /// stops at the first and the last row. With no selection, it selects
    /// the first row.
    pub fn move_selection(&mut self, step: isize) {
        let Some(last) = self.photos.len().checked_sub(1) else {
            return;
        };
        let ix = match self.selected.and_then(|id| self.index_of(id)) {
            Some(ix) => ix.saturating_add_signed(step).min(last),
            None => 0,
        };
        self.selected = Some(self.photos[ix].id);
    }

    /// Every photo's id and source, in list order, for a new run.
    pub fn queue(&self) -> Vec<(PhotoId, PathBuf)> {
        self.photos
            .iter()
            .map(|photo| (photo.id, photo.source.clone()))
            .collect()
    }

    /// Marks every photo as waiting and locks the list, for a new run.
    pub fn start(&mut self) {
        for photo in &mut self.photos {
            photo.status = Status::Waiting;
        }
        self.running = true;
    }

    /// Unlocks the list after a run.
    pub fn finish(&mut self) {
        self.running = false;
    }

    /// Sets the status of `id`. A photo removed since the run started is
    /// ignored.
    pub fn set_status(&mut self, id: PhotoId, status: Status) {
        if let Some(photo) = self.photos.iter_mut().find(|photo| photo.id == id) {
            photo.status = status;
        }
    }

    fn index_of(&self, id: PhotoId) -> Option<usize> {
        self.photos.iter().position(|photo| photo.id == id)
    }

    /// Replaces the list with `photos` as one undoable change.
    fn commit(&mut self, photos: Vec<Photo>) {
        self.history.replace_current(self.photos.clone());
        self.history.push(photos.clone());
        self.photos = photos;
    }

    /// Puts back a list from the history. The selection stays when its photo
    /// is back too, else it moves to the first row.
    fn restore(&mut self, photos: Vec<Photo>) -> bool {
        self.photos = photos;
        if self.selected.and_then(|id| self.index_of(id)).is_none() {
            self.selected = self.photos.first().map(Photo::id);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn batch(names: &[&str]) -> Batch {
        let mut batch = Batch::default();
        batch.add(paths(names));
        batch
    }

    fn ids(batch: &Batch) -> Vec<PhotoId> {
        batch.photos().iter().map(Photo::id).collect()
    }

    fn prepared() -> Status {
        Status::Prepared {
            output: PathBuf::from("out.jpg"),
        }
    }

    #[test]
    fn adding_a_photo_twice_keeps_one() {
        let mut batch = batch(&["a.jpg", "b.jpg"]);
        assert_eq!(batch.add(paths(&["b.jpg", "c.jpg"])), 1);
        let sources: Vec<_> = batch.photos().iter().map(Photo::source).collect();
        assert_eq!(sources, paths(&["a.jpg", "b.jpg", "c.jpg"]));
    }

    #[test]
    fn ids_stay_unique_after_removal() {
        let mut batch = batch(&["a.jpg", "b.jpg"]);
        let first = batch.photos()[0].id();
        batch.remove(first);
        batch.add(paths(&["a.jpg"]));
        let ids = ids(&batch);
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(!ids.contains(&first));
    }

    #[test]
    fn a_result_for_a_removed_photo_is_ignored() {
        let mut batch = batch(&["a.jpg"]);
        let queue = batch.queue();
        batch.clear();
        batch.set_status(queue[0].0, prepared());
        assert!(batch.is_empty());
    }

    #[test]
    fn the_first_photo_added_is_selected() {
        let mut batch = batch(&["a.jpg", "b.jpg"]);
        let [a, _] = ids(&batch)[..] else { panic!() };
        assert_eq!(batch.selected_id(), Some(a));
        batch.add(paths(&["c.jpg"]));
        assert_eq!(batch.selected_id(), Some(a), "a later add keeps it");
    }

    #[test]
    fn the_selection_moves_and_stops_at_the_ends() {
        let mut batch = batch(&["a.jpg", "b.jpg", "c.jpg"]);
        let [a, b, c] = ids(&batch)[..] else { panic!() };
        batch.move_selection(1);
        assert_eq!(batch.selected_id(), Some(b));
        batch.move_selection(1);
        batch.move_selection(1);
        assert_eq!(batch.selected_id(), Some(c));
        batch.move_selection(-1);
        batch.move_selection(-1);
        batch.move_selection(-1);
        assert_eq!(batch.selected_id(), Some(a));
        batch.select(c);
        assert_eq!(batch.selected().map(Photo::id), Some(c));
    }

    #[test]
    fn removing_the_selected_row_selects_the_next_one() {
        let mut batch = batch(&["a.jpg", "b.jpg", "c.jpg"]);
        let [_, b, c] = ids(&batch)[..] else { panic!() };
        batch.select(b);
        batch.remove_selected();
        assert_eq!(batch.selected_id(), Some(c));
        batch.remove_selected();
        assert_eq!(ids(&batch).len(), 1);
        assert_eq!(batch.selected_id(), Some(ids(&batch)[0]), "the last row");
        batch.remove_selected();
        assert_eq!(batch.selected_id(), None);
    }

    #[test]
    fn undo_and_redo_restore_add_remove_and_clear() {
        let mut batch = batch(&["a.jpg", "b.jpg"]);
        let both = ids(&batch);
        batch.remove(both[0]);
        batch.clear();
        assert!(batch.is_empty());

        assert!(batch.undo());
        assert_eq!(ids(&batch), [both[1]]);
        assert!(batch.undo());
        assert_eq!(ids(&batch), both);
        assert!(batch.undo());
        assert!(batch.is_empty(), "the add is undone");
        assert!(!batch.undo(), "nothing is left to undo");

        assert!(batch.redo());
        assert_eq!(ids(&batch), both);
        assert!(batch.redo());
        assert!(batch.redo());
        assert!(batch.is_empty());
        assert!(!batch.redo());
    }

    #[test]
    fn undoing_a_clear_brings_back_the_results() {
        let mut batch = batch(&["a.jpg", "b.jpg"]);
        let both = ids(&batch);
        batch.start();
        batch.set_status(both[0], prepared());
        batch.set_status(both[1], Status::Failed("bad".into()));
        batch.finish();

        batch.clear();
        batch.undo();
        let statuses: Vec<_> = batch.photos().iter().map(|p| p.status().clone()).collect();
        assert_eq!(statuses, [prepared(), Status::Failed("bad".into())]);
        assert_eq!(ids(&batch), both);
    }

    #[test]
    fn redo_keeps_results_that_came_in_after_an_undo() {
        let mut batch = batch(&["a.jpg"]);
        let a = ids(&batch)[0];
        batch.clear();
        batch.undo();
        batch.start();
        batch.set_status(a, prepared());
        batch.finish();
        batch.redo();
        batch.undo();
        assert_eq!(*batch.photos()[0].status(), prepared());
    }

    #[test]
    fn a_new_change_drops_the_redo() {
        let mut batch = batch(&["a.jpg"]);
        batch.clear();
        batch.undo();
        batch.add(paths(&["b.jpg"]));
        assert!(!batch.can_redo());
    }

    #[test]
    fn undo_keeps_fifty_changes() {
        let mut batch = Batch::default();
        for i in 0..60 {
            batch.add([PathBuf::from(format!("{i}.jpg"))]);
        }
        let mut undone = 0;
        while batch.undo() {
            undone += 1;
        }
        assert_eq!(undone, 50);
        assert_eq!(batch.len(), 10);
    }

    #[test]
    fn the_list_is_locked_during_a_run() {
        let mut batch = batch(&["a.jpg", "b.jpg"]);
        let both = ids(&batch);
        batch.start();
        assert_eq!(batch.add(paths(&["c.jpg"])), 0);
        batch.remove(both[0]);
        batch.clear();
        assert!(!batch.can_undo());
        assert!(!batch.undo());
        assert_eq!(ids(&batch), both);

        batch.set_status(both[0], Status::Preparing);
        assert_eq!(*batch.photos()[0].status(), Status::Preparing);
        batch.finish();
        assert!(batch.undo());
    }

    #[test]
    fn start_resets_the_results() {
        let mut batch = batch(&["a.jpg"]);
        let id = ids(&batch)[0];
        batch.set_status(id, prepared());
        batch.start();
        assert_eq!(*batch.photos()[0].status(), Status::Waiting);
    }

    #[test]
    fn show_in_finder_shows_the_output_once_prepared() {
        let mut batch = batch(&["a.jpg"]);
        let id = ids(&batch)[0];
        assert_eq!(batch.photos()[0].shown_file(), Path::new("a.jpg"));
        batch.set_status(id, prepared());
        assert_eq!(batch.photos()[0].shown_file(), Path::new("out.jpg"));
    }
}
