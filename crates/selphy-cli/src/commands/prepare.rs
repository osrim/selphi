//! `selphy prepare`: runs the batch with a progress bar and reports each photo.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};

use selphy::config::{self, Config};
use selphy::prepare::{self, Done, Job, Options};

use selphy::report::{photo_error, placement_summary};

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

/// Prepares the photos, reporting each on stdout and each failure on stderr.
/// Exits with failure when any photo failed.
pub fn run(args: PrepareArgs) -> Result<ExitCode> {
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

    let cfg = Config::load(&config::default_path())?;
    let inputs = prepare::collect_inputs(&paths)?;
    if inputs.is_empty() {
        println!("No images in {}", display_list(&paths));
        return Ok(ExitCode::SUCCESS);
    }

    let job = Job::new(
        cfg,
        Options {
            out_dir: args.out,
            archive_dir,
            camera_ref: args.camera_ref,
        },
    )?;
    let bar = ProgressBar::new(inputs.len() as u64);
    bar.set_style(ProgressStyle::with_template(
        "{bar:30} {pos}/{len}  {elapsed}",
    )?);
    let mut failed = 0;
    for source in &inputs {
        let result = job.run_one(source);
        // Print inside suspend: a line printed while the bar is drawn would
        // break up the bar's line.
        bar.suspend(|| match &result {
            Ok(done) => println!("{}", done_line(source, done)),
            Err(err) => {
                failed += 1;
                eprintln!("✗ {}  {}", source.display(), photo_error(err, source));
            }
        });
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
    println!("{summary}");
    if failed > 0 {
        eprintln!("{failed} failed and left in place");
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
