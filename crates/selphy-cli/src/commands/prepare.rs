//! `selphy prepare`: runs the batch on a few worker threads with a progress
//! bar, and reports each photo in input order.

use std::fmt::Write as _;
use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use anyhow::Result;
use clap::Args;
use indicatif::ProgressBar;

use selphy::config::{ConfigFile, FIT_ENV};
use selphy::geometry::{Fit, Placement};
use selphy::paper::Paper;
use selphy::prepare::{self, Done, Dry, Job, Options, Planned};
use selphy::report::{photo_error, placement_summary};

use crate::terminal::Terminal;

/// The most workers by default: each one holds a decoded photo, about 72 MB
/// for 24 MP.
const MAX_DEFAULT_JOBS: usize = 4;

#[derive(Args)]
pub struct PrepareArgs {
    /// Photos or folders to prepare [default: src]
    paths: Vec<PathBuf>,

    /// Where the print-ready JPEGs go.
    #[arg(short, long, default_value = "out")]
    out: PathBuf,

    /// Move finished sources here [default: originals, when reading src]
    #[arg(long, conflicts_with = "no_archive")]
    archive: Option<PathBuf>,

    /// Leave finished sources where they are.
    #[arg(long)]
    no_archive: bool,

    /// An unedited camera JPEG whose Exif is written into every output, for
    /// printers that reject edited files.
    #[arg(long, env = "CAMERA_REF")]
    camera_ref: Option<PathBuf>,

    /// How a photo fills the card [default: the config's `fit`, else contain]
    #[arg(long, env = FIT_ENV, value_enum)]
    fit: Option<Fit>,

    /// How many photos to prepare at once [default: the number of cores, at
    /// most 4]
    #[arg(short, long, value_name = "N")]
    jobs: Option<NonZeroUsize>,

    /// Show each photo's output, placement and archive path; write and move
    /// nothing.
    #[arg(long)]
    dry_run: bool,
}

/// Prepares the photos for the paper, reporting each on stdout and each
/// failure on stderr, in input order. Exits with failure when any photo
/// failed. An uncalibrated paper, and two sources that would write one
/// output, are errors before any photo is read.
///
/// A dry run reads only each photo's header, and reports what a real run
/// would do.
pub fn run(
    args: PrepareArgs,
    term: &mut Terminal,
    file: &ConfigFile,
    paper: Option<Paper>,
) -> Result<ExitCode> {
    // With no paths, work the default folders: src/ -> out/, archived to
    // originals/. Named paths are only archived when asked.
    let reading_src = args.paths.is_empty();
    let paths = if reading_src {
        vec![PathBuf::from("src")]
    } else {
        args.paths
    };
    let archive_dir = match (args.no_archive, args.archive) {
        (true, _) => None,
        (false, Some(dir)) => Some(dir),
        (false, None) => reading_src.then(|| PathBuf::from("originals")),
    };

    let loaded = file.load(paper, args.fit)?;
    let profile = loaded.profile()?;
    let inputs = prepare::collect_inputs(&paths)?;
    if inputs.is_empty() {
        writeln!(term.err, "No images in {}", display_list(&paths))?;
        return Ok(ExitCode::SUCCESS);
    }
    let planned = prepare::plan(&inputs, &args.out)?;

    let job = Job::new(
        loaded.paper,
        profile,
        Options {
            out_dir: args.out,
            archive_dir,
            camera_ref: args.camera_ref,
            fit: loaded.fit,
        },
    )?;
    let jobs = args.jobs.map_or_else(default_jobs, NonZeroUsize::get);
    let bar = term.progress(planned.len())?;
    let dry = |planned: &Planned| job.plan_one(planned).map(|dry| dry_line(planned, &dry));
    let real = |planned: &Planned| job.run_one(planned).map(|done| done_line(planned, &done));
    let work: &(dyn Fn(&Planned) -> Result<String> + Sync) =
        if args.dry_run { &dry } else { &real };
    let mut failed = 0;
    in_order(&planned, jobs, &bar, work, |planned, line| {
        // Print inside suspend: a line printed while the bar is drawn would
        // break up the bar's line.
        bar.suspend(|| match line {
            Ok(line) => writeln!(term.out, "{line}"),
            Err(err) => {
                failed += 1;
                let source = &planned.source;
                let reason = photo_error(&err, source);
                writeln!(term.err, "✗ {}  {reason}", source.display())
            }
        })?;
        Ok(())
    })?;
    bar.finish_and_clear();

    if args.dry_run {
        writeln!(term.out, "\nDry run: nothing written.")?;
        if failed > 0 {
            writeln!(term.err, "{failed} would fail")?;
            return Ok(ExitCode::FAILURE);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let opts = job.options();
    let mut summary = format!(
        "\n{} prepared → {}",
        planned.len() - failed,
        opts.out_dir.display()
    );
    if let Some(dir) = &opts.archive_dir {
        write!(summary, ", sources → {}", dir.display())?;
    }
    writeln!(term.out, "{summary}")?;
    if failed > 0 {
        writeln!(term.err, "{failed} failed and left in place")?;
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// The number of cores, at most [`MAX_DEFAULT_JOBS`].
fn default_jobs() -> usize {
    thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(MAX_DEFAULT_JOBS)
}

/// Runs `work` on each of `planned` with `jobs` threads, and calls `report`
/// on the results in the order of `planned`, on this thread. The bar
/// advances as each result arrives. When `report` fails, the workers take
/// no more photos, and the error is returned once they stop; the photos they
/// were on are finished, and not reported.
fn in_order<T: Send>(
    planned: &[Planned],
    jobs: usize,
    bar: &ProgressBar,
    work: impl Fn(&Planned) -> T + Sync,
    mut report: impl FnMut(&Planned, T) -> Result<()>,
) -> Result<()> {
    let next = AtomicUsize::new(0);
    let (sender, results) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..jobs.min(planned.len()) {
            let sender = sender.clone();
            let (next, work) = (&next, &work);
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = planned.get(index) else {
                        break;
                    };
                    if sender.send((index, work(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);

        // Results that arrived before an earlier one, kept until it is
        // reported.
        let mut waiting: Vec<Option<T>> = planned.iter().map(|_| None).collect();
        let mut reported = 0;
        for (index, result) in results {
            bar.inc(1);
            waiting[index] = Some(result);
            while let Some(result) = waiting.get_mut(reported).and_then(Option::take) {
                if let Err(err) = report(&planned[reported], result) {
                    next.store(planned.len(), Ordering::Relaxed);
                    return Err(err);
                }
                reported += 1;
            }
        }
        Ok(())
    })
}

/// The line for a prepared photo: its orientation, stretch, and white or
/// cut.
fn done_line(planned: &Planned, done: &Done) -> String {
    let source = &planned.source;
    let mut line = format!(
        "✓ {}  {}",
        source.display(),
        placement_text(&done.prepared.placement)
    );
    if let Some(archived) = &done.archived
        && archived.file_name() != source.file_name()
    {
        let _ = write!(line, " (archived as {})", archived.display());
    }
    line
}

/// The line for a photo in a dry run: its output, its placement as
/// [`done_line`] gives it, and where it would be archived.
fn dry_line(planned: &Planned, dry: &Dry) -> String {
    let mut line = format!(
        "→ {}  → {}  {}",
        planned.source.display(),
        planned.output.display(),
        placement_text(&dry.placement)
    );
    if let Some(archived) = &dry.archived {
        let _ = write!(line, "  (archive: {})", archived.display());
    }
    line
}

/// "portrait, stretched 1.9%, edge to edge".
fn placement_text(placement: &Placement) -> String {
    format!(
        "{}, {}",
        placement.canvas.orientation.name(),
        placement_summary(placement)
    )
}

fn display_list(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use selphy::test_util::{fresh_dir, write_jpeg};

    use super::*;

    fn args(paths: Vec<PathBuf>, dir: &Path) -> PrepareArgs {
        PrepareArgs {
            paths,
            out: dir.join("out"),
            archive: Some(dir.join("originals")),
            no_archive: false,
            camera_ref: None,
            fit: None,
            jobs: None,
            dry_run: false,
        }
    }

    /// `dir/src` with six photos of different shapes, `c.jpg` broken when
    /// `broken`.
    fn six_photos(dir: &Path, broken: bool) -> PathBuf {
        let src = dir.join("src");
        fs::create_dir(&src).unwrap();
        let shapes = [
            (300, 200),
            (200, 300),
            (320, 180),
            (200, 200),
            (400, 300),
            (180, 320),
        ];
        for (name, (width, height)) in ["a", "b", "c", "d", "e", "f"].into_iter().zip(shapes) {
            write_jpeg(&src.join(format!("{name}.jpg")), width, height, &[]);
        }
        if broken {
            fs::write(src.join("c.jpg"), b"not a jpeg").unwrap();
        }
        src
    }

    /// What `prepare` writes for six photos in a fresh `name` folder with
    /// `jobs` workers, with the folder's path written as `DIR`.
    fn six_with_jobs(name: &str, jobs: usize) -> (ExitCode, String) {
        let dir = fresh_dir(name);
        let src = six_photos(&dir, false);
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));
        let args = PrepareArgs {
            jobs: NonZeroUsize::new(jobs),
            ..args(vec![src], &dir)
        };
        let code = run(args, &mut term, &file, None).unwrap();
        (
            code,
            written.out().replace(&dir.display().to_string(), "DIR"),
        )
    }

    #[test]
    fn three_workers_print_what_one_prints_in_input_order() {
        let (one_code, one) = six_with_jobs("cli-prepare-jobs-1", 1);
        let (three_code, three) = six_with_jobs("cli-prepare-jobs-3", 3);
        assert_eq!(
            (one_code, three_code),
            (ExitCode::SUCCESS, ExitCode::SUCCESS)
        );
        assert_eq!(three, one);
        let names: Vec<&str> = one
            .lines()
            .filter_map(|line| line.strip_prefix("✓ DIR/src/"))
            .map(|rest| &rest[..5])
            .collect();
        assert_eq!(
            names,
            ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg", "f.jpg"]
        );
    }

    #[test]
    fn a_bad_photo_in_the_middle_fails_the_run_and_the_others_are_done() {
        let dir = fresh_dir("cli-prepare-jobs-bad");
        let src = six_photos(&dir, true);
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));
        let args = PrepareArgs {
            jobs: NonZeroUsize::new(3),
            ..args(vec![src.clone()], &dir)
        };

        let code = run(args, &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::FAILURE);
        assert!(written.out().contains("5 prepared → "), "{}", written.out());
        assert!(written.err().contains("1 failed and left in place"));
        assert!(src.join("c.jpg").exists());
        for name in ["a", "b", "d", "e", "f"] {
            assert!(
                dir.join(format!("out/{name}-selphy.jpg")).exists(),
                "{name}"
            );
            assert!(dir.join(format!("originals/{name}.jpg")).exists(), "{name}");
        }
    }

    #[test]
    fn two_sources_for_one_output_fail_before_any_photo_is_prepared() {
        let dir = fresh_dir("cli-prepare-clash");
        let a = write_jpeg(&dir.join("x.jpg"), 300, 200, &[]);
        let b = write_jpeg(&dir.join("x.png"), 300, 200, &[]);
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));

        let err = run(args(vec![dir.clone()], &dir), &mut term, &file, None).unwrap_err();

        assert_eq!(
            err.to_string(),
            format!(
                "{} and {} would both be written to {}",
                a.display(),
                b.display(),
                dir.join("out/x-selphy.jpg").display()
            )
        );
        assert_eq!(written.out(), "");
        assert!(!dir.join("out").exists());
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn a_dry_run_prints_the_plan_and_writes_nothing() {
        let dir = fresh_dir("cli-prepare-dry");
        let src = dir.join("src");
        fs::create_dir(&src).unwrap();
        let photo = write_jpeg(&src.join("a.jpg"), 320, 180, &[]);
        fs::write(src.join("bad.jpg"), b"not a jpeg").unwrap();
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));
        let dry = PrepareArgs {
            dry_run: true,
            ..args(vec![src.clone()], &dir)
        };

        let code = run(dry, &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::FAILURE);
        let (out, err) = (written.out(), written.err());
        let head = format!(
            "→ {}  → {}  ",
            photo.display(),
            dir.join("out/a-selphy.jpg").display()
        );
        let tail = format!("  (archive: {})\n", dir.join("originals/a.jpg").display());
        let placement = out
            .strip_prefix(&head)
            .and_then(|rest| rest.split_once(&tail))
            .map(|(placement, _)| placement.to_string())
            .unwrap_or_else(|| panic!("{out}"));
        assert!(out.ends_with("\nDry run: nothing written.\n"), "{out}");
        assert!(
            err.contains(&format!("✗ {}", src.join("bad.jpg").display())),
            "{err}"
        );
        assert!(err.ends_with("1 would fail\n"), "{err}");
        assert!(!dir.join("out").exists() && !dir.join("originals").exists());
        assert!(photo.exists());

        let (mut term, written) = Terminal::scripted([]);
        run(args(vec![photo.clone()], &dir), &mut term, &file, None).unwrap();
        let real = written.out();
        assert!(
            real.starts_with(&format!("✓ {}  {placement}\n", photo.display())),
            "{real}"
        );
    }

    #[test]
    fn a_good_photo_goes_to_stdout_and_a_bad_one_to_stderr_with_exit_1() {
        let dir = fresh_dir("cli-prepare");
        let src = dir.join("src");
        fs::create_dir(&src).unwrap();
        write_jpeg(&src.join("good.jpg"), 300, 200, &[]);
        fs::write(src.join("bad.jpg"), b"not a jpeg").unwrap();
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));

        let code = run(args(vec![src.clone()], &dir), &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::FAILURE);
        let (out, err) = (written.out(), written.err());
        assert!(
            out.contains(&format!("✓ {}", src.join("good.jpg").display())),
            "{out}"
        );
        assert!(out.contains("1 prepared → "), "{out}");
        assert!(!out.contains("bad.jpg"), "{out}");
        assert!(
            err.contains(&format!("✗ {}", src.join("bad.jpg").display())),
            "{err}"
        );
        assert!(err.contains("1 failed and left in place"), "{err}");
        assert!(dir.join("originals/good.jpg").exists());
        assert!(src.join("bad.jpg").exists());
    }

    #[test]
    fn an_empty_folder_says_so_on_stderr_and_exits_0() {
        let dir = fresh_dir("cli-prepare-empty");
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));

        let code = run(args(vec![dir.clone()], &dir), &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(written.out(), "");
        assert_eq!(written.err(), format!("No images in {}\n", dir.display()));
    }

    #[test]
    fn an_env_override_is_used() {
        let dir = fresh_dir("cli-prepare-override");
        let photo = write_jpeg(&dir.join("a.jpg"), 300, 200, &[]);
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"))
            .with_overrides([("SELPHY_MAX_STRETCH_PCT", "0")]);

        let code = run(args(vec![photo], &dir), &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(
            written.out().contains("stretched 0.0%"),
            "{}",
            written.out()
        );
    }

    #[test]
    fn fit_cover_cuts_a_wide_photo() {
        let dir = fresh_dir("cli-prepare-cover");
        let photo = write_jpeg(&dir.join("wide.jpg"), 320, 180, &[]);
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));
        let args = PrepareArgs {
            fit: Some(Fit::Cover),
            ..args(vec![photo], &dir)
        };

        let code = run(args, &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        let out = written.out();
        assert!(
            out.contains("landscape, stretched 2.4%, cut left "),
            "{out}"
        );
        assert!(!out.contains("white"), "{out}");
    }

    #[test]
    fn the_config_files_fit_is_used() {
        let dir = fresh_dir("cli-prepare-cover-config");
        let photo = write_jpeg(&dir.join("wide.jpg"), 320, 180, &[]);
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));
        fs::write(file.path(), "fit = \"cover\"\n").unwrap();

        run(args(vec![photo], &dir), &mut term, &file, None).unwrap();

        assert!(written.out().contains("cut left "), "{}", written.out());
    }

    #[test]
    fn an_uncalibrated_paper_fails_with_the_calibrate_hint() {
        let dir = fresh_dir("cli-prepare-uncalibrated");
        let photo = write_jpeg(&dir.join("a.jpg"), 300, 200, &[]);
        let (mut term, written) = Terminal::scripted([]);
        let file = ConfigFile::at(dir.join("printer.toml"));

        let err = run(
            args(vec![photo.clone()], &dir),
            &mut term,
            &file,
            Some(Paper::L),
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "l paper is not calibrated. Run: selphy calibrate --paper l"
        );
        assert_eq!(written.out(), "");
        assert!(photo.exists(), "the source is not archived");
        assert!(!dir.join("out").exists());
    }
}
