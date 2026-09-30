//! The `prepare` job: turns photos into print-ready JPEGs one file at a time,
//! so that one bad file never stops the batch. [`plan`] names each output
//! first, and refuses a batch in which two sources would write one output.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use image::RgbImage;
use image::imageops::{self, FilterType};

use crate::atomic;
use crate::config::Profile;
use crate::geometry::{self, Fit, Placement};
use crate::imaging::{self, Look, Source};
use crate::record::Record;

mod archive;
mod inputs;

use archive::{archive, free_name};
pub use inputs::{Planned, collect_inputs, plan};

/// What preparing one photo produced.
#[derive(Debug)]
pub struct Prepared {
    /// The print-ready JPEG.
    pub output: PathBuf,
    /// Where the picture went on the canvas.
    pub placement: Placement,
}

/// Where a job reads and writes.
#[derive(Debug, Clone)]
pub struct Options {
    /// Where the print-ready JPEGs go.
    pub out_dir: PathBuf,
    /// Where finished sources are moved. `None` leaves them in place.
    pub archive_dir: Option<PathBuf>,
    /// An unedited camera JPEG whose Exif is written into every output.
    pub camera_ref: Option<PathBuf>,
    /// How each photo fills the safe box. The placement record says which.
    pub fit: Fit,
    /// The sharpening and the background of each output.
    pub look: Look,
}

/// A photo that was prepared, and archived when asked.
#[derive(Debug)]
pub struct Done {
    /// The output and its placement.
    pub prepared: Prepared,
    /// Where the source went, when archiving.
    pub archived: Option<PathBuf>,
}

/// How one photo would print, drawn small for a screen.
#[derive(Debug)]
pub struct Preview {
    /// The rendered canvas, scaled down to the requested side.
    pub image: RgbImage,
    /// The whole placed picture at the same scale, including what a cover
    /// placement puts past the canvas edge, which `image` clips off.
    pub picture: RgbImage,
    /// Where the picture goes on the canvas.
    pub placement: Placement,
    /// The source's width and height, the right way up.
    pub source_size: (u32, u32),
}

/// One source rendered onto its canvas.
struct Rendered {
    /// The canvas, as the output holds it.
    sheet: RgbImage,
    placement: Placement,
    /// The source in sRGB, the right way up.
    photo: RgbImage,
    /// The source's width and height, the right way up.
    source_size: (u32, u32),
}

/// What preparing one photo would do, from its header only.
#[derive(Debug)]
pub struct Dry {
    /// Where the picture would go on the canvas.
    pub placement: Placement,
    /// Where the source would be moved, when archiving, as the archive
    /// folder is now.
    pub archived: Option<PathBuf>,
}

/// One configured prepare run. `new` checks what it can before the first
/// photo; `run_one` then prepares one planned source per call, so that the
/// caller decides the order, the progress display, and when to stop.
///
/// A job is `Send + Sync`, so threads can share one by reference. A clone
/// shares the camera Exif, so a clone is cheap to move to another thread.
#[derive(Debug, Clone)]
pub struct Job {
    profile: Profile,
    opts: Options,
    camera_exif: Option<Arc<[u8]>>,
}

impl Job {
    /// A job that prepares photos with `profile`, as `opts` says. Reads the
    /// camera reference's Exif once, when one is given; an unreadable
    /// reference, or one with no Exif, is an error, so that no photo gets the
    /// wrong Exif.
    pub fn new(profile: Profile, opts: Options) -> Result<Self> {
        let camera_exif = opts
            .camera_ref
            .as_deref()
            .map(camera_exif)
            .transpose()?
            .map(Arc::from);
        Ok(Self {
            profile,
            opts,
            camera_exif,
        })
    }

    /// Where this job reads and writes.
    pub fn options(&self) -> &Options {
        &self.opts
    }

    /// Prepares the planned source into its output, then archives it when
    /// the job has an archive folder. A failed photo is not archived, so it
    /// stays where it was for inspection.
    pub fn run_one(&self, planned: &Planned) -> Result<Done> {
        let prepared = self.prepare(planned)?;
        let archived = match &self.opts.archive_dir {
            // Named, so that an archive failure does not read like a failed
            // prepare once the source path is left out.
            Some(dir) => Some(
                archive(&planned.source, dir)
                    .with_context(|| format!("archiving to {}", dir.display()))?,
            ),
            None => None,
        };
        Ok(Done { prepared, archived })
    }

    /// What `run_one` would do with the planned source, without writing or
    /// moving anything. Reads only the photo's header, so a photo whose
    /// pixels or colour profile are broken passes here and fails in
    /// `run_one`.
    pub fn plan_one(&self, planned: &Planned) -> Result<Dry> {
        let (width, height) = imaging::probe(&planned.source)?;
        let placement = geometry::place(&self.profile, width, height, self.opts.fit)
            .context("the image is empty")?;
        let archived = self
            .opts
            .archive_dir
            .as_deref()
            .map(|dir| free_name(dir, &planned.source))
            .transpose()?;
        Ok(Dry {
            placement,
            archived,
        })
    }

    /// How `source` would print: the canvas `run_one` would write, from the
    /// same pipeline, scaled down so that its longer side is `max_side_px`.
    /// Nothing is encoded or written.
    pub fn preview(&self, source: &Path, max_side_px: u32) -> Result<Preview> {
        let Rendered {
            sheet,
            placement,
            photo,
            source_size,
        } = self.render(source)?;
        let (width, height) = sheet.dimensions();
        let scale = imaging::fit_within(width, height, max_side_px)
            .map_or(1.0, |(w, _)| f64::from(w) / f64::from(width));
        let scaled = |px: i64| ((px as f64 * scale).round() as u32).max(1);
        let image = imageops::resize(
            &sheet,
            scaled(width.into()),
            scaled(height.into()),
            FilterType::Triangle,
        );
        let picture = imageops::resize(
            &photo,
            scaled(placement.width),
            scaled(placement.height),
            FilterType::Triangle,
        );
        Ok(Preview {
            image,
            picture,
            placement,
            source_size,
        })
    }

    fn render(&self, source: &Path) -> Result<Rendered> {
        let Source {
            image, icc_profile, ..
        } = imaging::load(source)?;
        let source_size = (image.width(), image.height());
        let placement = geometry::place(&self.profile, source_size.0, source_size.1, self.opts.fit)
            .context("the image is empty")?;
        let photo = imaging::to_srgb(image, icc_profile.as_deref())?;
        Ok(Rendered {
            sheet: imaging::render(&photo, &placement, self.opts.look),
            placement,
            photo,
            source_size,
        })
    }

    /// Prepares the planned source and writes it to its output. The photo's
    /// own Exif is not carried over: it names the editing software,
    /// which some printers reject, and its resolution tags contradict the
    /// 300 dpi header. The camera reference's Exif, when given, is written
    /// instead.
    fn prepare(&self, planned: &Planned) -> Result<Prepared> {
        let Rendered {
            sheet, placement, ..
        } = self.render(&planned.source)?;
        let record = Record::of(&placement);
        let jpeg = imaging::encode_jpeg(&sheet, self.camera_exif.as_deref(), &[record.segment()])?;

        let output = planned.output.clone();
        if let Some(out_dir) = output.parent() {
            fs::create_dir_all(out_dir)
                .with_context(|| format!("creating {}", out_dir.display()))?;
        }
        atomic::write(&output, jpeg)?;
        Ok(Prepared { output, placement })
    }
}

/// The camera reference's Exif, with its orientation reset like any source's.
fn camera_exif(path: &Path) -> Result<Vec<u8>> {
    let context = || format!("reading the camera reference {}", path.display());
    imaging::load(path)
        .with_context(context)?
        .exif
        .with_context(|| format!("{}: no Exif block", context()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper::Paper;
    use crate::test_util::postcard;
    use crate::test_util::{exif_with_orientation, fresh_dir, write_jpeg};

    fn source_jpeg(dir: &Path, name: &str, exif: &[u8]) -> PathBuf {
        write_jpeg(&dir.join(name), 300, 200, exif)
    }

    fn exif_of(path: &Path) -> Option<Vec<u8>> {
        use image::ImageDecoder;
        let mut decoder = image::ImageReader::open(path)
            .unwrap()
            .into_decoder()
            .unwrap();
        decoder.exif_metadata().unwrap()
    }

    #[test]
    fn prepares_a_print_ready_jpeg() {
        let dir = fresh_dir("prepare-one");
        let source = source_jpeg(&dir, "photo.v2.jpg", &exif_with_orientation(1));
        let out = dir.join("out");

        let done = job(&dir, false).prepare(&planned(&dir, &source)).unwrap();
        assert_eq!(done.output, out.join("photo.v2-selphy.jpg"));
        let written = image::open(&done.output).unwrap();
        assert_eq!((written.width(), written.height()), (1772, 1181));
        assert_eq!(
            exif_of(&done.output),
            None,
            "the photo's own Exif is dropped"
        );
    }

    #[test]
    fn camera_exif_is_written_when_given() {
        let dir = fresh_dir("prepare-camera");
        let source = source_jpeg(&dir, "photo.jpg", &exif_with_orientation(1));
        let camera = exif_with_orientation(1)
            .into_iter()
            .chain(*b"camera")
            .collect::<Vec<u8>>();

        let job = Job {
            camera_exif: Some(Arc::from(camera.clone())),
            ..job(&dir, false)
        };
        let done = job.prepare(&planned(&dir, &source)).unwrap();
        assert_eq!(exif_of(&done.output), Some(camera));
    }

    #[test]
    fn an_unreadable_photo_is_an_error_not_a_crash() {
        let dir = fresh_dir("prepare-bad");
        let bad = dir.join("broken.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let out = dir.join("out");
        let err = job(&dir, false).prepare(&planned(&dir, &bad)).unwrap_err();
        assert!(format!("{err:#}").contains("broken.jpg"), "{err:#}");
        assert!(!out.exists(), "nothing is written for a failed photo");
    }

    /// The plan for `source` into `dir/out`, where [`options`] writes.
    fn planned(dir: &Path, source: &Path) -> Planned {
        let mut planned = plan(&[source.to_path_buf()], &dir.join("out")).unwrap();
        planned.remove(0)
    }

    fn options(dir: &Path, archive: bool) -> Options {
        Options {
            out_dir: dir.join("out"),
            archive_dir: archive.then(|| dir.join("originals")),
            camera_ref: None,
            fit: Fit::Contain,
            look: Look::default(),
        }
    }

    fn job(dir: &Path, archive: bool) -> Job {
        Job::new(postcard(), options(dir, archive)).unwrap()
    }

    #[test]
    fn a_bad_photo_does_not_stop_the_batch() {
        let dir = fresh_dir("batch");
        let good_a = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let bad = dir.join("b.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let good_c = source_jpeg(&dir, "c.jpg", &exif_with_orientation(1));

        let job = job(&dir, true);
        let results: Vec<_> = [&good_a, &bad, &good_c]
            .into_iter()
            .map(|source| job.run_one(&planned(&dir, source)))
            .collect();

        let ok: Vec<bool> = results.iter().map(Result::is_ok).collect();
        assert_eq!(ok, [true, false, true]);
        assert!(bad.exists(), "a failed photo stays in place");
        assert!(
            !good_a.exists() && !good_c.exists(),
            "finished photos are archived"
        );
        let done = results[0].as_ref().unwrap();
        assert_eq!(done.archived, Some(dir.join("originals/a.jpg")));
    }

    #[test]
    fn an_archive_failure_says_it_was_the_archive() {
        let dir = fresh_dir("batch-archive-bad");
        let photo = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        fs::write(dir.join("originals"), b"a file, not a folder").unwrap();

        let err = job(&dir, true).run_one(&planned(&dir, &photo)).unwrap_err();
        assert!(err.to_string().starts_with("archiving to "), "{err:#}");
        assert!(
            photo.exists(),
            "a source that was not archived stays in place"
        );
    }

    #[test]
    fn without_an_archive_dir_sources_stay_put() {
        let dir = fresh_dir("batch-no-archive");
        let photo = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let done = job(&dir, false).run_one(&planned(&dir, &photo)).unwrap();
        assert_eq!(done.archived, None);
        assert!(photo.exists());
    }

    #[test]
    fn camera_reference_exif_goes_into_every_output() {
        let dir = fresh_dir("batch-camera");
        let a = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let b = source_jpeg(&dir, "b.jpg", &exif_with_orientation(1));
        // A camera shot tagged "rotate 90": its orientation must be reset, or
        // the printer would turn every photo sideways.
        let camera = source_jpeg(&dir, "camera.jpg", &exif_with_orientation(6));
        let opts = Options {
            camera_ref: Some(camera),
            ..options(&dir, false)
        };
        let job = Job::new(postcard(), opts).unwrap();
        for source in [a, b] {
            let output = job
                .run_one(&planned(&dir, &source))
                .unwrap()
                .prepared
                .output;
            assert_eq!(exif_of(&output), Some(exif_with_orientation(1)));
        }
    }

    #[test]
    fn an_unreadable_camera_reference_fails_the_job() {
        let dir = fresh_dir("batch-camera-bad");
        let opts = Options {
            camera_ref: Some(dir.join("missing.jpg")),
            ..options(&dir, true)
        };
        let err = Job::new(postcard(), opts).unwrap_err();
        assert!(format!("{err:#}").contains("camera reference"), "{err:#}");
    }

    #[test]
    fn a_clone_prepares_the_same_output() {
        let dir = fresh_dir("batch-clone");
        let a = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let b = source_jpeg(&dir, "b.jpg", &exif_with_orientation(1));
        let camera = source_jpeg(&dir, "camera.jpg", &exif_with_orientation(1));
        let opts = Options {
            camera_ref: Some(camera),
            ..options(&dir, false)
        };
        let job = Job::new(postcard(), opts).unwrap();
        let clone = job.clone();

        let from_job = job.run_one(&planned(&dir, &a)).unwrap().prepared.output;
        let b = planned(&dir, &b);
        let from_clone = std::thread::spawn(move || clone.run_one(&b).unwrap().prepared.output)
            .join()
            .unwrap();
        assert_eq!(from_clone.parent(), from_job.parent());
        assert_eq!(fs::read(from_clone).unwrap(), fs::read(from_job).unwrap());
    }

    #[test]
    fn the_record_names_the_paper_and_canvas() {
        let dir = fresh_dir("batch-paper");
        let photo = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let output = job(&dir, false)
            .run_one(&planned(&dir, &photo))
            .unwrap()
            .prepared
            .output;
        let record = Record::read(&output).unwrap();
        assert_eq!(record.paper, Paper::Postcard);
        assert_eq!(record.canvas_px, (1772, 1181));
    }

    #[test]
    fn plan_one_gives_the_real_placement_and_writes_nothing() {
        let dir = fresh_dir("batch-dry");
        let photo = write_jpeg(&dir.join("a.jpg"), 320, 180, &exif_with_orientation(6));
        fs::create_dir(dir.join("originals")).unwrap();
        fs::write(dir.join("originals/a.jpg"), b"archived earlier").unwrap();
        let job = job(&dir, true);
        let planned = planned(&dir, &photo);

        let dry = job.plan_one(&planned).unwrap();

        assert_eq!(dry.archived, Some(dir.join("originals/a-2.jpg")));
        assert!(!dir.join("out").exists(), "nothing is written");
        assert!(photo.exists(), "the source stays in place");
        let done = job.run_one(&planned).unwrap();
        assert_eq!(dry.placement, done.prepared.placement);
        assert_eq!(done.archived, dry.archived);
    }

    #[test]
    fn plan_one_reports_a_broken_file() {
        let dir = fresh_dir("batch-dry-bad");
        let bad = dir.join("broken.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let err = job(&dir, false).plan_one(&planned(&dir, &bad)).unwrap_err();
        assert!(format!("{err:#}").contains("broken.jpg"), "{err:#}");
    }

    #[test]
    fn a_preview_places_the_photo_as_run_one_does_and_writes_nothing() {
        let dir = fresh_dir("batch-preview");
        let photo = write_jpeg(&dir.join("a.jpg"), 320, 180, &exif_with_orientation(6));
        for fit in [Fit::Contain, Fit::Cover] {
            let opts = Options {
                fit,
                ..options(&dir, false)
            };
            let job = Job::new(postcard(), opts).unwrap();

            let preview = job.preview(&photo, 400).unwrap();
            assert!(!dir.join("out").exists(), "{fit}: nothing is written");
            assert_eq!(preview.source_size, (180, 320), "{fit}");

            let done = job.run_one(&planned(&dir, &photo)).unwrap();
            assert_eq!(preview.placement, done.prepared.placement, "{fit}");
            fs::remove_dir_all(dir.join("out")).unwrap();
        }
    }

    #[test]
    fn a_preview_has_the_canvas_aspect_and_fits_the_side() {
        let dir = fresh_dir("batch-preview-size");
        let photo = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let preview = job(&dir, false).preview(&photo, 300).unwrap();
        let canvas = &preview.placement.canvas;
        // 1772 x 1181 scaled to 300 on the long side.
        assert_eq!(preview.image.dimensions(), (300, 200));
        let scale = 300.0 / 1772.0;
        let (pw, ph) = preview.picture.dimensions();
        assert!((f64::from(pw) - preview.placement.width as f64 * scale).abs() <= 1.0);
        assert!((f64::from(ph) - preview.placement.height as f64 * scale).abs() <= 1.0);
        let aspect = |w: f64, h: f64| w / h;
        let (w, h) = preview.image.dimensions();
        assert!(
            (aspect(f64::from(w), f64::from(h))
                - aspect(canvas.width as f64, canvas.height as f64))
            .abs()
                < 0.01
        );
    }

    #[test]
    fn a_preview_of_a_broken_photo_is_an_error() {
        let dir = fresh_dir("batch-preview-bad");
        let bad = dir.join("broken.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let err = job(&dir, false).preview(&bad, 300).unwrap_err();
        assert!(format!("{err:#}").contains("broken.jpg"), "{err:#}");
    }

    #[test]
    fn a_job_can_be_shared_by_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Job>();
    }

    #[test]
    fn a_cover_job_fills_the_card_and_records_cover() {
        let dir = fresh_dir("batch-cover");
        let photo = write_jpeg(&dir.join("wide.jpg"), 320, 180, &exif_with_orientation(1));
        let opts = Options {
            fit: Fit::Cover,
            ..options(&dir, false)
        };
        let job = Job::new(postcard(), opts).unwrap();
        let prepared = job.run_one(&planned(&dir, &photo)).unwrap().prepared;
        assert_eq!(prepared.placement.fit, Fit::Cover);
        assert_eq!(Record::read(&prepared.output).unwrap().fit, Fit::Cover);
    }
}
