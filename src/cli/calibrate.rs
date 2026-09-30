//! `selphy calibrate`: writes the bracket sheet, then turns the numbers read
//! off the printed card into trims.

use std::fmt;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, ValueEnum};
use inquire::{Confirm, Select};

use selphy::calibrate::{self, CANDIDATES_MM};
use selphy::config::{self, Config};
use selphy::geometry::{Edge, Orientation, Trim};

use super::report::{answer, confirm_save, print_changes};

#[derive(Args)]
pub struct CalibrateArgs {
    /// The orientation of the sheet. Either one measures all four trims.
    #[arg(long, value_enum, default_value_t = SheetOrientation::Landscape)]
    orientation: SheetOrientation,

    /// Only write the sheet; do not ask for readings.
    #[arg(long, conflicts_with = "read")]
    sheet_only: bool,

    /// Do not write the sheet; go straight to entering readings from a print.
    #[arg(long)]
    read: bool,

    /// Where to write the sheet [default: calibration-<orientation>.jpg]
    #[arg(short, long, conflicts_with = "read")]
    out: Option<PathBuf>,

    /// The TrueType font for the labels.
    #[arg(long, env = "SELPHY_FONT", default_value = calibrate::DEFAULT_FONT)]
    font: PathBuf,
}

#[derive(Clone, Copy, ValueEnum)]
enum SheetOrientation {
    Landscape,
    Portrait,
}

impl From<SheetOrientation> for Orientation {
    fn from(value: SheetOrientation) -> Self {
        match value {
            SheetOrientation::Landscape => Orientation::Landscape,
            SheetOrientation::Portrait => Orientation::Portrait,
        }
    }
}

/// Writes the bracket sheet, then asks for the readings and offers to save
/// the trims, as the flags select.
pub fn run(args: CalibrateArgs) -> Result<ExitCode> {
    let orientation = Orientation::from(args.orientation);
    let path = config::default_path();
    let cfg = Config::load(&path)?;
    let later = format!(
        "selphy calibrate --read --orientation {}",
        orientation.name()
    );

    if !args.read {
        let out = args
            .out
            .unwrap_or_else(|| PathBuf::from(format!("calibration-{}.jpg", orientation.name())));
        let font = calibrate::load_font(&args.font)?;
        calibrate::write_sheet(&cfg, orientation, &font, &out)?;
        println!("Wrote {}", out.display());
        println!("Print it Borderless and tear the tabs. On each edge, find the");
        println!("smallest number whose line still shows.");
        if args.sheet_only {
            return Ok(ExitCode::SUCCESS);
        }
        if !std::io::stdin().is_terminal() {
            println!("\nNot a terminal. To enter the readings, run: {later}");
            return Ok(ExitCode::SUCCESS);
        }
        let ready = Confirm::new("Printed it, and ready to enter the readings?")
            .with_default(false)
            .prompt();
        if !answer(ready)? {
            println!("When you have the print, run: {later}");
            return Ok(ExitCode::SUCCESS);
        }
    }

    let readings = ask_readings(&cfg, orientation)?;
    if readings.is_empty() {
        println!("No readings; nothing changed.");
        return Ok(ExitCode::SUCCESS);
    }
    let updated = calibrate::apply_readings(&cfg, orientation, &readings);
    print_changes(&cfg, &updated, orientation);

    confirm_save(&updated, &path)?;
    Ok(ExitCode::SUCCESS)
}

/// One choice in the per-edge prompt.
enum Reading {
    Line(f64),
    NoneVisible,
    Skip,
}

impl fmt::Display for Reading {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Reading::Line(mm) => write!(f, "{mm:.1}"),
            Reading::NoneVisible => write!(f, "no line visible"),
            Reading::Skip => write!(f, "skip this edge (keep its trim)"),
        }
    }
}

fn ask_readings(cfg: &Config, orientation: Orientation) -> Result<Vec<(Edge, f64)>> {
    let help = format!(
        "hold the card {} as printed; the cursor starts at the current trim",
        orientation.name()
    );
    let mut readings = Vec::new();
    for edge in Edge::ALL {
        let current = Trim::at(orientation, edge).mm(cfg);
        let options: Vec<Reading> = CANDIDATES_MM
            .iter()
            .map(|&mm| Reading::Line(mm))
            .chain([Reading::NoneVisible, Reading::Skip])
            .collect();
        let message = format!(
            "{} edge: smallest number whose line still shows",
            edge.name()
        );
        let choice = Select::new(&message, options)
            .with_help_message(&help)
            .with_starting_cursor(nearest_candidate(current))
            .prompt();
        let [.., largest] = CANDIDATES_MM;
        match answer(choice)? {
            Reading::Line(mm) => readings.push((edge, mm)),
            Reading::NoneVisible => println!(
                "  The {} edge trims more than {largest} mm, beyond this sheet; its trim is kept.",
                edge.name(),
            ),
            Reading::Skip => {}
        }
    }
    Ok(readings)
}

/// The index of the candidate closest to `mm`.
fn nearest_candidate(mm: f64) -> usize {
    CANDIDATES_MM
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - mm).abs().total_cmp(&(*b - mm).abs()))
        .map_or(0, |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_starts_at_the_closest_candidate() {
        assert_eq!(nearest_candidate(1.5), 0);
        assert_eq!(nearest_candidate(2.1), 1); // 2.0
        assert_eq!(nearest_candidate(2.7), 2); // 2.5
        assert_eq!(nearest_candidate(9.0), CANDIDATES_MM.len() - 1);
    }

    #[test]
    fn readings_display_as_the_printed_labels() {
        assert_eq!(Reading::Line(2.0).to_string(), "2.0");
        assert_eq!(Reading::NoneVisible.to_string(), "no line visible");
    }
}
