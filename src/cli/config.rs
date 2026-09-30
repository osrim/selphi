//! `selphy config`: shows the printer geometry and its effect on common
//! photo shapes.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;

use selphy::config::{self, Config};
use selphy::geometry::{self, Edge, Orientation, Trim};

use super::report::white_summary;

#[derive(Args)]
pub struct ConfigArgs {
    /// Print only the config file's path.
    #[arg(long)]
    path: bool,

    /// Write the current values to the config file, for editing by hand.
    #[arg(long, conflicts_with = "path")]
    init: bool,
}

/// Photo shapes shown by `selphy config`, as landscape pixel sizes.
const SAMPLE_SHAPES: [(&str, u32, u32); 4] = [
    ("3:2", 3000, 2000),
    ("4:3", 4000, 3000),
    ("16:9", 1920, 1080),
    ("1:1", 2000, 2000),
];

/// Prints the config file's path, writes the file, or shows the geometry, as
/// the flags select.
pub fn run(args: ConfigArgs) -> Result<ExitCode> {
    let path = config::default_path();
    if args.path {
        println!("{}", path.display());
        return Ok(ExitCode::SUCCESS);
    }
    let exists = path.try_exists()?;
    let cfg = Config::load(&path)?;
    if args.init {
        if exists {
            bail!("{} already exists", path.display());
        }
        cfg.save(&path)?;
        println!("Wrote {}", path.display());
        return Ok(ExitCode::SUCCESS);
    }

    let status = if exists {
        ""
    } else {
        "  (not found: using defaults)"
    };
    println!("Config  {}{status}", path.display());
    println!(
        "Canvas  {} x {} mm, stretch up to {}%\n",
        cfg.canvas_long_mm, cfg.canvas_short_mm, cfg.max_stretch_pct
    );

    println!("Trim, mm    landscape  portrait");
    for edge in Edge::ALL {
        let trim = |orientation| Trim::at(orientation, edge).mm(&cfg);
        println!(
            "  {:<10}{:>9.1}{:>10.1}",
            edge.name(),
            trim(Orientation::Landscape),
            trim(Orientation::Portrait)
        );
    }

    println!("\nHow a photo lands");
    for (shape, width, height) in SAMPLE_SHAPES {
        let placement = geometry::place(&cfg, width, height).expect("sample sizes are non-zero");
        println!(
            "  {shape:<6}stretched {:.1}%, {}",
            placement.stretch_pct,
            white_summary(&placement)
        );
    }
    Ok(ExitCode::SUCCESS)
}
