//! `selphy prepare`: runs the batch with a progress bar and reports each photo.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;

use selphy::config::ConfigFile;
use selphy::paper::Paper;
use selphy::prepare::{self, Done, Job, Options};
use selphy::report::{photo_error, placement_summary};

use crate::terminal::Terminal;

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
}

/// Prepares the photos for the paper, reporting each on stdout and each
/// failure on stderr. Exits with failure when any photo failed. An
/// uncalibrated paper is an error before any photo is read.
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

    let loaded = file.load(paper)?;
    let profile = loaded.profile()?;
    let inputs = prepare::collect_inputs(&paths)?;
    if inputs.is_empty() {
        writeln!(term.err, "No images in {}", display_list(&paths))?;
        return Ok(ExitCode::SUCCESS);
    }

    let job = Job::new(
        loaded.paper,
        profile,
        Options {
            out_dir: args.out,
            archive_dir,
            camera_ref: args.camera_ref,
        },
    )?;
    let bar = term.progress(inputs.len())?;
    let mut failed = 0;
    for source in &inputs {
        let result = job.run_one(source);
        // Print inside suspend: a line printed while the bar is drawn would
        // break up the bar's line.
        bar.suspend(|| match &result {
            Ok(done) => writeln!(term.out, "{}", done_line(source, done)),
            Err(err) => {
                failed += 1;
                let reason = photo_error(err, source);
                writeln!(term.err, "✗ {}  {reason}", source.display())
            }
        })?;
        bar.inc(1);
    }
    bar.finish_and_clear();

    let opts = job.options();
    let mut summary = format!(
        "\n{} prepared → {}",
        inputs.len() - failed,
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

/// The line for a prepared photo: its orientation, stretch, and white.
fn done_line(source: &Path, done: &Done) -> String {
    let name = source.display();
    let p = &done.prepared.placement;
    let mut line = format!(
        "✓ {name}  {}, {}",
        p.canvas.orientation.name(),
        placement_summary(p)
    );
    if let Some(archived) = &done.archived
        && archived.file_name() != source.file_name()
    {
        let _ = write!(line, " (archived as {})", archived.display());
    }
    line
}

fn display_list(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use selphy::test_util::{fresh_dir, write_jpeg};

    use super::*;

    fn args(paths: Vec<PathBuf>, dir: &Path) -> PrepareArgs {
        PrepareArgs {
            paths,
            out: dir.join("out"),
            archive: Some(dir.join("originals")),
            no_archive: false,
            camera_ref: None,
        }
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
