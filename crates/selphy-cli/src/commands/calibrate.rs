//! `selphy calibrate`: writes the bracket sheet, then turns the numbers read
//! off the printed card into trims.

use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, ValueEnum};

use selphy::calibrate::{self, CANDIDATES_MM};
use selphy::config::{Config, ConfigFile};
use selphy::geometry::{Edge, Orientation, Trim};

use crate::terminal::{Terminal, confirm_save, print_changes};

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
/// the trims, as the flags select. The sheet uses the values this run uses;
/// the readings are applied to the file's values.
pub fn run(args: CalibrateArgs, term: &mut Terminal, file: &ConfigFile) -> Result<ExitCode> {
    let orientation = Orientation::from(args.orientation);
    let loaded = file.load()?;
    let later = format!(
        "selphy calibrate --read --orientation {}",
        orientation.name()
    );

    if !args.read {
        let out = args
            .out
            .unwrap_or_else(|| PathBuf::from(format!("calibration-{}.jpg", orientation.name())));
        let font = calibrate::load_font(&args.font)?;
        calibrate::write_sheet(&loaded.effective, orientation, &font, &out)?;
        writeln!(term.out, "Wrote {}", out.display())?;
        writeln!(
            term.out,
            "Print it Borderless and tear the tabs. On each edge, find the"
        )?;
        writeln!(term.out, "smallest number whose line still shows.")?;
        if args.sheet_only {
            return Ok(ExitCode::SUCCESS);
        }
        if !term.is_interactive {
            writeln!(
                term.err,
                "\nNot a terminal. To enter the readings, run: {later}"
            )?;
            return Ok(ExitCode::SUCCESS);
        }
        if !term.confirm("Printed it, and ready to enter the readings?", false)? {
            writeln!(term.out, "When you have the print, run: {later}")?;
            return Ok(ExitCode::SUCCESS);
        }
    }

    let readings = ask_readings(term, &loaded.saved, orientation)?;
    if readings.is_empty() {
        writeln!(term.out, "No readings; nothing changed.")?;
        return Ok(ExitCode::SUCCESS);
    }
    let updated = loaded.saved.with_trims(orientation, &readings)?;
    print_changes(term, &loaded.saved, &updated, orientation)?;

    confirm_save(term, &loaded, &updated, file)?;
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

/// Asks, for each edge of a printed sheet, for the smallest number whose
/// line still shows. That number is the top of the bracket the trim lies
/// in, so a picture fitted with it is never cropped. The sheet's
/// orientation decides which trim each edge measures.
fn ask_readings(
    term: &mut Terminal,
    cfg: &Config,
    orientation: Orientation,
) -> Result<Vec<(Edge, f64)>> {
    let help = format!(
        "hold the card {} as printed; the cursor starts at the current trim",
        orientation.name()
    );
    let choices: Vec<Reading> = CANDIDATES_MM
        .iter()
        .map(|&mm| Reading::Line(mm))
        .chain([Reading::NoneVisible, Reading::Skip])
        .collect();
    let labels: Vec<String> = choices.iter().map(Reading::to_string).collect();
    let mut readings = Vec::new();
    for edge in Edge::ALL {
        let current = Trim::at(orientation, edge).mm(cfg);
        let message = format!(
            "{} edge: smallest number whose line still shows",
            edge.name()
        );
        let choice = term.select(&message, &labels, nearest_candidate(current), &help)?;
        let [.., largest] = CANDIDATES_MM;
        match choices[choice] {
            Reading::Line(mm) => readings.push((edge, mm)),
            Reading::NoneVisible => writeln!(
                term.out,
                "  The {} edge trims more than {largest} mm, beyond this sheet; its trim is kept.",
                edge.name(),
            )?,
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

    use std::fs;

    use selphy::test_util::fresh_dir;

    use crate::terminal::{Answer, Cancelled};

    const SKIP: usize = CANDIDATES_MM.len() + 1;

    fn read_args() -> CalibrateArgs {
        CalibrateArgs {
            orientation: SheetOrientation::Landscape,
            sheet_only: false,
            read: true,
            out: None,
            font: PathBuf::from(calibrate::DEFAULT_FONT),
        }
    }

    /// The index of the choice for the line at `mm`.
    fn line(mm: f64) -> Answer {
        Answer::Select(CANDIDATES_MM.iter().position(|&c| c == mm).unwrap())
    }

    #[test]
    fn read_shows_the_changes_and_saves_on_yes() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-yes").join("printer.toml"));
        let answers = [
            line(3.0),
            Answer::Select(SKIP),
            Answer::Select(SKIP),
            Answer::Select(SKIP),
            Answer::Confirm(true),
        ];
        let (mut term, written) = Terminal::scripted(answers);
        let code = run(read_args(), &mut term, &file).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        let out = written.out();
        assert!(
            out.contains("  left                4.50   3.00  *\n"),
            "{out}"
        );
        assert!(out.contains("  top                 2.10   2.10\n"), "{out}");
        assert!(out.ends_with("Saved.\n"), "{out}");
        assert_eq!(file.load().unwrap().saved.trim_long_a_mm, 3.0);
    }

    #[test]
    fn read_does_not_save_on_no_and_exits_0() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-no").join("printer.toml"));
        let answers = [
            line(3.0),
            line(2.0),
            Answer::Select(SKIP),
            Answer::Select(SKIP),
            Answer::Confirm(false),
        ];
        let (mut term, written) = Terminal::scripted(answers);
        assert_eq!(
            run(read_args(), &mut term, &file).unwrap(),
            ExitCode::SUCCESS
        );
        assert!(written.out().ends_with("Not saved.\n"), "{}", written.out());
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn a_cancel_is_a_cancelled_error_and_saves_nothing() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-cancel").join("printer.toml"));
        let (mut term, _) = Terminal::scripted([line(3.0), Answer::Cancel]);
        let err = run(read_args(), &mut term, &file).unwrap_err();
        assert!(err.is::<Cancelled>(), "{err:#}");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn no_readings_change_nothing() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-skip").join("printer.toml"));
        let (mut term, written) = Terminal::scripted([Answer::Select(SKIP); 4]);
        assert_eq!(
            run(read_args(), &mut term, &file).unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(written.out(), "No readings; nothing changed.\n");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn an_override_on_a_saved_trim_is_warned_about_and_not_saved() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-env").join("printer.toml"))
            .with_overrides([
                ("SELPHY_TRIM_LONG_A_MM", "5"),
                ("SELPHY_TRIM_SHORT_A_MM", "1"),
            ]);
        let answers = [
            line(3.0),
            Answer::Select(SKIP),
            Answer::Select(SKIP),
            Answer::Select(SKIP),
            Answer::Confirm(true),
        ];
        let (mut term, written) = Terminal::scripted(answers);
        run(read_args(), &mut term, &file).unwrap();
        // Only long A is saved; short A keeps its file value.
        assert_eq!(
            written.err(),
            "SELPHY_TRIM_LONG_A_MM is set; the saved value is not used while it is\n"
        );
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 3.0"), "{text}");
        assert!(text.contains("trim_short_a_mm = 2.1"), "{text}");
    }

    #[test]
    fn readings_display_as_the_printed_labels() {
        assert_eq!(Reading::Line(2.0).to_string(), "2.0");
        assert_eq!(Reading::NoneVisible.to_string(), "no line visible");
    }
}
