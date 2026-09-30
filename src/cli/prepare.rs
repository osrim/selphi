//! `selphy prepare`: runs the batch with a progress bar and reports each photo.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};

use selphy::config::{self, Config};
use selphy::prepare::{self, Options, Outcome};

use super::report::{error_chain, white_summary};

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

    let opts = Options {
        out_dir: args.out,
        archive_dir,
        camera_ref: args.camera_ref,
    };
    let bar = ProgressBar::new(inputs.len() as u64);
    bar.set_style(ProgressStyle::with_template(
        "{bar:30} {pos}/{len}  {elapsed}",
    )?);
    let outcomes = prepare::prepare_batch(&inputs, &cfg, &opts, |outcome| {
        // suspend, not println: println prints nothing when stdout is piped.
        bar.suspend(|| println!("{}", outcome_line(outcome)));
        bar.inc(1);
    })?;
    bar.finish_and_clear();

    let failed = outcomes.iter().filter(|o| o.result.is_err()).count();
    let mut summary = format!(
        "\n{} prepared → {}",
        outcomes.len() - failed,
        opts.out_dir.display()
    );
    if let Some(dir) = &opts.archive_dir {
        summary.push_str(&format!(", sources → {}", dir.display()));
    }
    println!("{summary}");
    if failed > 0 {
        println!("{failed} failed and left in place");
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// One line per photo: what was done, or why it failed.
fn outcome_line(outcome: &Outcome) -> String {
    let name = outcome.source.display();
    let done = match &outcome.result {
        Ok(done) => done,
        Err(err) => return format!("✗ {name}  {}", error_chain(err)),
    };
    let p = &done.prepared.placement;
    let mut line = format!(
        "✓ {name}  {}, stretched {:.1}%, {}",
        p.canvas.orientation.name(),
        p.stretch_pct,
        white_summary(p)
    );
    if let Some(archived) = &done.archived
        && archived.file_name() != outcome.source.file_name()
    {
        line.push_str(&format!(" (archived as {})", archived.display()));
    }
    line
}

fn display_list(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    names.join(", ")
}
