//! The `prepare` job: turns photos into print-ready JPEGs one file at a time,
//! so that one bad file never stops the batch.

use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::atomic;
use crate::config::Config;
use crate::geometry::{self, Placement};
use crate::imaging::{self, Source};

/// The formats the decoder is built with (see the `image` features in
/// Cargo.toml).
const EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "tif", "tiff"];

/// The photos to prepare: every file named in `paths`, plus the supported
/// images directly inside every directory named (not recursive). Sorted, and
/// each file listed once.
pub fn collect_inputs(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in paths {
        if !path.is_dir() {
            files.push(path.clone());
            continue;
        }
        let entries = fs::read_dir(path).with_context(|| format!("listing {}", path.display()))?;
        for entry in entries {
            let file = entry?.path();
            if file.is_file() && is_supported(&file) {
                files.push(file);
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// A supported extension, in any case, on a file that is not hidden. Hidden
/// files include the `._name.jpg` metadata files macOS writes on external
/// drives, which are not images.
fn is_supported(path: &Path) -> bool {
    let hidden = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'));
    let known = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        });
    known && !hidden
}

/// What preparing one photo produced.
#[derive(Debug)]
pub struct Prepared {
    /// The print-ready JPEG.
    pub output: PathBuf,
    /// Where the picture went on the canvas.
    pub placement: Placement,
}

/// Prepares one photo and writes it to `out_dir/<name>.jpg`. The photo's own
/// Exif is not carried over: it names the editing software, which some
/// printers reject, and its resolution tags contradict the 300 dpi header.
/// `camera_exif`, when given, is written instead.
pub fn prepare_one(
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

/// `photo.v2.png` becomes `photo.v2.jpg`: only the last extension is
/// replaced.
fn output_name(source: &Path) -> Result<OsString> {
    let mut name = source
        .file_stem()
        .with_context(|| format!("{} has no file name", source.display()))?
        .to_os_string();
    name.push(".jpg");
    Ok(name)
}

/// Where a batch reads and writes.
#[derive(Debug, Clone)]
pub struct Options {
    /// Where the print-ready JPEGs go.
    pub out_dir: PathBuf,
    /// Where finished sources are moved. `None` leaves them in place.
    pub archive_dir: Option<PathBuf>,
    /// An unedited camera JPEG whose Exif is written into every output.
    pub camera_ref: Option<PathBuf>,
}

/// What happened to one input.
#[derive(Debug)]
pub struct Outcome {
    /// The input as given.
    pub source: PathBuf,
    /// What was done, or why it failed.
    pub result: Result<Done>,
}

/// A photo that was prepared, and archived when asked.
#[derive(Debug)]
pub struct Done {
    /// The output and its placement.
    pub prepared: Prepared,
    /// Where the source went, when archiving.
    pub archived: Option<PathBuf>,
}

/// Prepares every input, in order. A failed photo is reported in its
/// `Outcome` and the batch goes on; it is not archived, so it stays where it
/// was for inspection. `on_progress` is called after each photo.
///
/// Fails as a whole only if the camera reference cannot be read.
pub fn prepare_batch(
    inputs: &[PathBuf],
    cfg: &Config,
    opts: &Options,
    mut on_progress: impl FnMut(&Outcome),
) -> Result<Vec<Outcome>> {
    let camera_exif = opts.camera_ref.as_deref().map(camera_exif).transpose()?;

    let mut outcomes = Vec::with_capacity(inputs.len());
    for source in inputs {
        let outcome = Outcome {
            source: source.clone(),
            result: prepare_and_archive(source, cfg, opts, camera_exif.as_deref()),
        };
        on_progress(&outcome);
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

fn prepare_and_archive(
    source: &Path,
    cfg: &Config,
    opts: &Options,
    camera_exif: Option<&[u8]>,
) -> Result<Done> {
    let prepared = prepare_one(source, &opts.out_dir, cfg, camera_exif)?;
    let archived = match &opts.archive_dir {
        Some(dir) => Some(archive(source, dir)?),
        None => None,
    };
    Ok(Done { prepared, archived })
}

/// The camera reference's Exif, with its orientation reset like any source's.
fn camera_exif(path: &Path) -> Result<Vec<u8>> {
    let context = || format!("reading the camera reference {}", path.display());
    imaging::load(path)
        .with_context(context)?
        .exif
        .with_context(|| format!("{}: no Exif block", context()))
}

/// Moves a finished source into `archive_dir` and returns where it went. An
/// existing file there is never replaced: `photo.jpg` becomes `photo-2.jpg`,
/// then `photo-3.jpg`, and so on.
pub fn archive(source: &Path, archive_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(archive_dir)
        .with_context(|| format!("creating {}", archive_dir.display()))?;
    let target = free_name(archive_dir, source)?;
    match fs::rename(source, &target) {
        Ok(()) => {}
        // A rename cannot cross disks, e.g. src/ on an SD card.
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {
            atomic::copy(source, &target)?;
            fs::remove_file(source).with_context(|| format!("removing {}", source.display()))?;
        }
        Err(e) => {
            return Err(e).with_context(|| format!("moving to {}", target.display()));
        }
    }
    Ok(target)
}

/// The first of `name.ext`, `name-2.ext`, `name-3.ext`, ... that does not
/// exist in `dir`.
fn free_name(dir: &Path, source: &Path) -> Result<PathBuf> {
    let stem = source
        .file_stem()
        .with_context(|| format!("{} has no file name", source.display()))?;
    let mut n = 1;
    let target = loop {
        let mut name = stem.to_os_string();
        if n > 1 {
            name.push(format!("-{n}"));
        }
        if let Some(ext) = source.extension() {
            name.push(".");
            name.push(ext);
        }
        let candidate = dir.join(name);
        if !candidate.try_exists()? {
            break candidate;
        }
        n += 1;
    };
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{exif_with_orientation, fresh_dir};

    #[test]
    fn collects_supported_images_from_a_directory() {
        let dir = fresh_dir("collect");
        for name in [
            "b.jpg",
            "A.JPEG",
            "c.tif",
            "notes.txt",
            ".hidden.jpg",
            "._b.jpg",
        ] {
            fs::write(dir.join(name), b"").unwrap();
        }
        fs::create_dir(dir.join("nested.jpg")).unwrap(); // a directory, not a file

        let names: Vec<String> = collect_inputs(&[dir])
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.JPEG", "b.jpg", "c.tif"]);
    }

    #[test]
    fn named_files_are_kept_as_given_and_listed_once() {
        let dir = fresh_dir("collect-named");
        let photo = dir.join("photo.jpg");
        fs::write(&photo, b"").unwrap();
        let odd = dir.join("scan.bmp"); // unsupported extension, but asked for
        let files = collect_inputs(&[photo.clone(), dir.clone(), odd.clone()]).unwrap();
        assert_eq!(files, [photo, odd]);
    }

    #[test]
    fn a_missing_directory_is_just_a_missing_file() {
        let paths = [PathBuf::from("/nonexistent/src")];
        assert_eq!(collect_inputs(&paths).unwrap(), paths);
    }

    /// Writes a small grey JPEG with the given Exif block, returns its path.
    fn source_jpeg(dir: &Path, name: &str, exif: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut encoder = jpeg_encoder::Encoder::new_file(&path, 90).unwrap();
        encoder.add_exif_metadata(exif).unwrap();
        let pixels = vec![128u8; 300 * 200 * 3];
        encoder
            .encode(&pixels, 300, 200, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        path
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
        assert_eq!(done.output, out.join("photo.v2.jpg"));
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

    #[test]
    fn archive_never_replaces_an_existing_file() {
        let dir = fresh_dir("archive");
        let (src, originals) = (dir.join("src"), dir.join("originals"));
        fs::create_dir(&src).unwrap();

        let mut archived = Vec::new();
        for round in 1..=3 {
            let photo = src.join("photo.v2.jpg");
            fs::write(&photo, format!("round {round}")).unwrap();
            archived.push(archive(&photo, &originals).unwrap());
            assert!(!photo.exists(), "the source is moved, not copied");
        }

        let names: Vec<_> = archived.iter().map(|p| p.file_name().unwrap()).collect();
        assert_eq!(names, ["photo.v2.jpg", "photo.v2-2.jpg", "photo.v2-3.jpg"]);
        let first = fs::read_to_string(&archived[0]).unwrap();
        assert_eq!(first, "round 1", "the first archived file is untouched");
    }

    #[test]
    fn archive_numbers_files_without_an_extension() {
        let dir = fresh_dir("archive-no-ext");
        let originals = dir.join("originals");
        fs::create_dir(&originals).unwrap();
        fs::write(originals.join("scan"), b"old").unwrap();
        let scan = dir.join("scan");
        fs::write(&scan, b"new").unwrap();
        assert_eq!(
            archive(&scan, &originals).unwrap(),
            originals.join("scan-2")
        );
    }

    fn options(dir: &Path, archive: bool) -> Options {
        Options {
            out_dir: dir.join("out"),
            archive_dir: archive.then(|| dir.join("originals")),
            camera_ref: None,
        }
    }

    #[test]
    fn a_bad_photo_does_not_stop_the_batch() {
        let dir = fresh_dir("batch");
        let good_a = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let bad = dir.join("b.jpg");
        fs::write(&bad, b"not a jpeg").unwrap();
        let good_c = source_jpeg(&dir, "c.jpg", &exif_with_orientation(1));
        let inputs = [good_a.clone(), bad.clone(), good_c.clone()];

        let mut seen = Vec::new();
        let outcomes = prepare_batch(&inputs, &Config::default(), &options(&dir, true), |o| {
            seen.push(o.source.clone())
        })
        .unwrap();

        assert_eq!(
            seen, inputs,
            "progress is reported for every photo, in order"
        );
        let ok: Vec<bool> = outcomes.iter().map(|o| o.result.is_ok()).collect();
        assert_eq!(ok, [true, false, true]);
        assert!(bad.exists(), "a failed photo stays in place");
        assert!(
            !good_a.exists() && !good_c.exists(),
            "finished photos are archived"
        );
        let done = outcomes[0].result.as_ref().unwrap();
        assert_eq!(done.archived, Some(dir.join("originals/a.jpg")));
    }

    #[test]
    fn without_an_archive_dir_sources_stay_put() {
        let dir = fresh_dir("batch-no-archive");
        let photo = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let outcomes = prepare_batch(
            std::slice::from_ref(&photo),
            &Config::default(),
            &options(&dir, false),
            |_| {},
        )
        .unwrap();
        assert_eq!(outcomes[0].result.as_ref().unwrap().archived, None);
        assert!(photo.exists());
    }

    #[test]
    fn camera_reference_exif_goes_into_every_output() {
        let dir = fresh_dir("batch-camera");
        let photo = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        // A camera shot tagged "rotate 90": its orientation must be reset, or
        // the printer would turn every photo sideways.
        let camera = source_jpeg(&dir, "camera.jpg", &exif_with_orientation(6));
        let opts = Options {
            camera_ref: Some(camera),
            ..options(&dir, false)
        };
        let outcomes = prepare_batch(&[photo], &Config::default(), &opts, |_| {}).unwrap();
        let output = &outcomes[0].result.as_ref().unwrap().prepared.output;
        assert_eq!(exif_of(output), Some(exif_with_orientation(1)));
    }

    #[test]
    fn an_unreadable_camera_reference_fails_the_whole_batch() {
        let dir = fresh_dir("batch-camera-bad");
        let photo = source_jpeg(&dir, "a.jpg", &exif_with_orientation(1));
        let opts = Options {
            camera_ref: Some(dir.join("missing.jpg")),
            ..options(&dir, true)
        };
        let err = prepare_batch(
            std::slice::from_ref(&photo),
            &Config::default(),
            &opts,
            |_| {},
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("camera reference"), "{err:#}");
        assert!(photo.exists(), "nothing was processed");
    }
}
