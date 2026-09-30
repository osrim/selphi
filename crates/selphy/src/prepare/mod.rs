//! The `prepare` job: turns photos into print-ready JPEGs one file at a time,
//! so that one bad file never stops the batch.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::atomic;
use crate::config::Config;
use crate::geometry::{self, Placement};
use crate::imaging::{self, Source};

mod archive;
mod inputs;

use archive::archive;
pub use inputs::collect_inputs;

/// What preparing one photo produced.
#[derive(Debug)]
pub struct Prepared {
    /// The print-ready JPEG.
    pub output: PathBuf,
    /// Where the picture went on the canvas.
    pub placement: Placement,
}

/// Prepares one photo and writes it to `out_dir/<name>-selphy.jpg`. The photo's own
/// Exif is not carried over: it names the editing software, which some
/// printers reject, and its resolution tags contradict the 300 dpi header.
/// `camera_exif`, when given, is written instead.
fn prepare_one(
    source: &Path,
    out_dir: &Path,
    cfg: &Config,
    camera_exif: Option<&[u8]>,
) -> Result<Prepared> {
    let Source {
        image, icc_profile, ..
    } = imaging::load(source)?;
    let placement =
        geometry::place(cfg, image.width(), image.height()).context("the image is empty")?;
    let photo = imaging::to_srgb(image, icc_profile.as_deref())?;
    let sheet = imaging::render(&photo, &placement);
    let jpeg = imaging::encode_jpeg(&sheet, &placement, camera_exif)?;

    let output = out_dir.join(output_name(source)?);
    fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    atomic::write(&output, jpeg)?;
    Ok(Prepared { output, placement })
}

/// `photo.v2.png` becomes `photo.v2-selphy.jpg`: only the last extension is
/// replaced.
fn output_name(source: &Path) -> Result<OsString> {
    let mut name = source
        .file_stem()
        .with_context(|| format!("{} has no file name", source.display()))?
        .to_os_string();
    name.push("-selphy.jpg");
    Ok(name)
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
}

/// A photo that was prepared, and archived when asked.
#[derive(Debug)]
pub struct Done {
    /// The output and its placement.
    pub prepared: Prepared,
    /// Where the source went, when archiving.
    pub archived: Option<PathBuf>,
}

/// One configured prepare run. `new` checks what it can before the first
/// photo; `run_one` then prepares one source per call, so that the caller
/// decides the order, the progress display, and when to stop.
///
/// A clone shares the camera Exif, so a clone is cheap to move to another
/// thread.
#[derive(Debug, Clone)]
pub struct Job {
    cfg: Config,
    opts: Options,
    camera_exif: Option<Arc<[u8]>>,
}

impl Job {
    /// A job that prepares photos with `cfg` as `opts` says. Reads the camera
    /// reference's Exif once, when one is given; an unreadable reference, or
    /// one with no Exif, is an error, so that no photo gets the wrong Exif.
    pub fn new(cfg: Config, opts: Options) -> Result<Self> {
        let camera_exif = opts
            .camera_ref
            .as_deref()
            .map(camera_exif)
            .transpose()?
            .map(Arc::from);
        Ok(Self {
            cfg,
            opts,
            camera_exif,
        })
    }

    /// Where this job reads and writes.
    pub fn options(&self) -> &Options {
        &self.opts
    }

    /// Prepares `source` into the output folder, then archives it when the
    /// job has an archive folder. A failed photo is not archived, so it stays
    /// where it was for inspection.
    pub fn run_one(&self, source: &Path) -> Result<Done> {
        let prepared = prepare_one(
            source,
            &self.opts.out_dir,
            &self.cfg,
            self.camera_exif.as_deref(),
        )?;
        let archived = match &self.opts.archive_dir {
            // Named, so that an archive failure does not read like a failed
            // prepare once the source path is left out.
            Some(dir) => Some(
                archive(source, dir).with_context(|| format!("archiving to {}", dir.display()))?,
            ),
            None => None,
        };
        Ok(Done { prepared, archived })
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
    use crate::test_util::{exif_with_orientation, fresh_dir, write_jpeg};

    /// Writes a 300x200 grey JPEG with the given Exif block, returns its path.
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

        let done = prepare_one(&source, &out, &Config::default(), None).unwrap();
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

        let done = prepare_one(&source, &dir, &Config::default(), Some(&camera)).unwrap();
        assert_eq!(exif_of(&done.output), Some(camera));
    }

    #[test]
    fn an_unreadable_photo_is_an_error_not_a_crash() {
        let dir = fresh_dir("prepare-bad");
        let bad = dir.join("broken.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let out = dir.join("out");
        let err = prepare_one(&bad, &out, &Config::default(), None).unwrap_err();
        assert!(format!("{err:#}").contains("broken.jpg"), "{err:#}");
        assert!(!out.exists(), "nothing is written for a failed photo");
    }

    fn options(dir: &Path, archive: bool) -> Options {
        Options {
            out_dir: dir.join("out"),
            archive_dir: archive.then(|| dir.join("originals")),
            camera_ref: None,
        }
    }

    fn job(dir: &Path, archive: bool) -> Job {
        Job::new(Config::default(), options(dir, archive)).unwrap()
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
            .map(|source| job.run_one(source))
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

        let err = job(&dir, true).run_one(&photo).unwrap_err();
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
        let done = job(&dir, false).run_one(&photo).unwrap();
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
        let job = Job::new(Config::default(), opts).unwrap();
        for source in [a, b] {
            let output = job.run_one(&source).unwrap().prepared.output;
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
        let err = Job::new(Config::default(), opts).unwrap_err();
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
        let job = Job::new(Config::default(), opts).unwrap();
        let clone = job.clone();

        let from_job = job.run_one(&a).unwrap().prepared.output;
        let from_clone = std::thread::spawn(move || clone.run_one(&b).unwrap().prepared.output)
            .join()
            .unwrap();
        assert_eq!(from_clone.parent(), from_job.parent());
        assert_eq!(fs::read(from_clone).unwrap(), fs::read(from_job).unwrap());
    }
}
