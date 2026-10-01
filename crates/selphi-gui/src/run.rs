//! One Prepare run: the planned photos, the next one to hand out, the cancel
//! flag and the counts. Plain data: the main window starts a few workers,
//! and each takes the next photo from here until there is none, so that the
//! photos are dispatched in list order.

use std::path::{Path, PathBuf};
use std::thread;

use anyhow::Result;
use selphi::prepare::{self, Planned};

use crate::batch::PhotoId;

/// The most photos prepared at once. Each worker holds a decoded photo,
/// about 72 MB for 24 MP, as the command line's `-j` default does.
const MAX_WORKERS: usize = 4;

/// The number of workers for a run: the CPUs, at most [`MAX_WORKERS`].
pub fn default_workers() -> usize {
    thread::available_parallelism().map_or(1, |n| n.get().min(MAX_WORKERS))
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub prepared: usize,
    pub failed: usize,
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct Run {
    queue: Vec<(PhotoId, Planned)>,
    /// The index of the next photo to hand out.
    next: usize,
    /// Photos handed out and not yet finished.
    active: usize,
    prepared: usize,
    failed: usize,
    cancelled: bool,
}

impl Run {
    /// The run for `photos` into `out_dir`. Fails, before any photo is
    /// prepared, when two photos would write the same output.
    pub fn plan(photos: Vec<(PhotoId, PathBuf)>, out_dir: &Path) -> Result<Self> {
        let (ids, sources): (Vec<_>, Vec<_>) = photos.into_iter().unzip();
        // The plan keeps the order of `sources`, so it pairs with `ids`.
        let planned = prepare::plan(&sources, out_dir)?;
        Ok(Self {
            queue: ids.into_iter().zip(planned).collect(),
            next: 0,
            active: 0,
            prepared: 0,
            failed: 0,
            cancelled: false,
        })
    }

    /// The next photo to prepare, in list order. `None` when every photo has
    /// been handed out, or after Cancel.
    pub fn take(&mut self) -> Option<(PhotoId, Planned)> {
        if self.cancelled {
            return None;
        }
        let item = self.queue.get(self.next)?.clone();
        self.next += 1;
        self.active += 1;
        Some(item)
    }

    /// Counts a photo that was handed out as done.
    pub fn record(&mut self, prepared: bool) {
        self.active = self.active.saturating_sub(1);
        if prepared {
            self.prepared += 1;
        } else {
            self.failed += 1;
        }
    }

    /// Stops handing out photos. The photos in progress still finish and
    /// are counted.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// No photo is in progress and none will be handed out.
    pub fn is_done(&self) -> bool {
        self.active == 0 && (self.cancelled || self.next >= self.queue.len())
    }

    /// The photos finished, and the photos in the run.
    pub fn progress(&self) -> (usize, usize) {
        (self.prepared + self.failed, self.queue.len())
    }

    pub fn summary(&self) -> Summary {
        Summary {
            prepared: self.prepared,
            failed: self.failed,
            cancelled: self.cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::batch::Batch;

    use super::*;

    /// A run of `names` from a batch, so that the ids are real.
    fn run(names: &[&str]) -> (Run, Vec<PhotoId>) {
        let mut batch = Batch::default();
        batch.add(names.iter().map(PathBuf::from));
        let queue = batch.queue();
        let ids = queue.iter().map(|(id, _)| *id).collect();
        (Run::plan(queue, Path::new("out")).unwrap(), ids)
    }

    #[test]
    fn photos_are_handed_out_in_list_order_and_counted() {
        let (mut run, ids) = run(&["a.jpg", "b.jpg", "c.jpg"]);
        let (first, planned) = run.take().unwrap();
        assert_eq!(first, ids[0]);
        assert_eq!(planned.output, Path::new("out/a-selphi.jpg"));
        let (second, _) = run.take().unwrap();
        assert_eq!(second, ids[1]);
        assert_eq!(run.progress(), (0, 3));

        run.record(true);
        run.record(false);
        assert_eq!(run.progress(), (2, 3));
        assert!(!run.is_done());
        assert_eq!(run.take().unwrap().0, ids[2]);
        assert_eq!(run.take(), None);
        run.record(true);
        assert!(run.is_done());
        assert_eq!(
            run.summary(),
            Summary {
                prepared: 2,
                failed: 1,
                cancelled: false
            }
        );
    }

    #[test]
    fn cancel_stops_the_dispatch_and_keeps_the_results_in_progress() {
        let (mut run, _) = run(&["a.jpg", "b.jpg", "c.jpg"]);
        run.take().unwrap();
        run.take().unwrap();
        run.cancel();
        assert_eq!(run.take(), None);
        assert!(!run.is_done(), "two photos are still in progress");
        run.record(true);
        run.record(true);
        assert!(run.is_done());
        assert_eq!(run.progress(), (2, 3));
        assert_eq!(
            run.summary(),
            Summary {
                prepared: 2,
                failed: 0,
                cancelled: true
            }
        );
    }

    #[test]
    fn two_photos_with_one_output_stop_the_start() {
        let mut batch = Batch::default();
        batch.add([PathBuf::from("x/a.jpg"), PathBuf::from("y/a.jpg")]);
        let err = Run::plan(batch.queue(), Path::new("out")).unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.contains("x/a.jpg") && text.contains("y/a.jpg"),
            "{text}"
        );
    }

    #[test]
    fn workers_are_at_most_four() {
        assert!((1..=MAX_WORKERS).contains(&default_workers()));
    }
}
