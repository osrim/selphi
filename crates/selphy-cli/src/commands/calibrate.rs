//! `selphy calibrate`: writes the bracket sheet, then turns the numbers read
//! off the printed card into trims.

use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{ArgGroup, Args, ValueEnum};

use selphy::calibrate::{self, CANDIDATES_MM};
use selphy::config::{ConfigFile, Profile};
use selphy::geometry::{Edge, Fit, Orientation, Trim};
use selphy::paper::Paper;

use crate::commands::EdgeArgs;
use crate::terminal::{Change, Terminal, confirm_save, print_changes};

#[derive(Args)]
#[command(group(
    ArgGroup::new("edges")
        .args(["left", "top", "right", "bottom"])
        .multiple(true)
        .requires("read")
))]
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

    // The readings: on each edge, the smallest number whose line still shows.
    #[command(flatten, next_help_heading = "Readings without prompts (need --read)")]
    edges: EdgeArgs,
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

/// Writes the paper's bracket sheet, then asks for the readings and offers
/// to save the trims to the paper's table, as the flags select. The other
/// papers' tables are kept. Readings given as edge flags are not asked for,
/// and each must be a line on the sheet; `--yes` saves without asking.
///
/// The paper's profile is its table, else its starting profile. The sheet
/// uses that profile with the overrides applied, as this run uses it; the
/// readings are applied to it without them.
pub fn run(
    args: CalibrateArgs,
    term: &mut Terminal,
    file: &ConfigFile,
    paper: Option<Paper>,
) -> Result<ExitCode> {
    let orientation = Orientation::from(args.orientation);
    // Sheets have no picture, so the fit is not read.
    let loaded = file.load(paper, Some(Fit::Contain))?;
    let paper = loaded.paper;
    let before = loaded
        .saved
        .profile(paper)
        .unwrap_or_else(|| paper.starting_profile());
    let later = format!(
        "selphy calibrate --read --paper {paper} --orientation {}",
        orientation.name()
    );

    if !args.read {
        let out = args
            .out
            .unwrap_or_else(|| PathBuf::from(format!("calibration-{}.jpg", orientation.name())));
        let font = calibrate::load_font(&args.font)?;
        let sheet = loaded.apply_overrides(before.clone())?;
        calibrate::write_sheet(paper, &sheet, orientation, &font, &out)?;
        writeln!(term.out, "Wrote {} for {paper} paper", out.display())?;
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

    let readings = args
        .edges
        .edges_or_ask(term, |term| ask_readings(term, &before, orientation))?;
    for &(_, mm) in &readings {
        check_line(mm)?;
    }
    if readings.is_empty() {
        writeln!(term.out, "No readings; nothing changed.")?;
        return Ok(ExitCode::SUCCESS);
    }
    let change = Change {
        after: before.with_trims(orientation, &readings)?,
        before,
        orientation,
    };
    print_changes(term, &change)?;
    confirm_save(term, &loaded, &change, file, args.edges.yes)?;
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
    profile: &Profile,
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
        let current = Trim::at(orientation, edge).mm(profile);
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

/// Fails when `mm` is not one of the lines on the sheet.
fn check_line(mm: f64) -> Result<()> {
    if CANDIDATES_MM.iter().any(|line| (line - mm).abs() < 1e-9) {
        return Ok(());
    }
    let lines: Vec<String> = CANDIDATES_MM.iter().map(|mm| format!("{mm:.1}")).collect();
    bail!(
        "{mm} mm is not a line on the sheet; use one of {}",
        lines.join(", ")
    )
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
            edges: EdgeArgs::none(),
        }
    }

    /// `--read` with the readings as flags.
    fn flag_args(left: Option<f64>, top: Option<f64>, yes: bool) -> CalibrateArgs {
        CalibrateArgs {
            edges: EdgeArgs {
                left,
                top,
                yes,
                ..EdgeArgs::none()
            },
            ..read_args()
        }
    }

    #[test]
    fn flags_and_yes_save_without_a_prompt_and_keep_the_other_edges() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-flags").join("printer.toml"));
        let (mut term, written) = Terminal::scripted([]);

        let code = run(
            flag_args(Some(3.0), Some(2.5), true),
            &mut term,
            &file,
            None,
        )
        .unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(written.out().ends_with("Saved.\n"), "{}", written.out());
        let saved = file.load(None, None).unwrap().saved_profile().unwrap();
        let start = Paper::Postcard.starting_profile();
        // landscape left = long A, top = short A
        assert_eq!(saved.trim_long_a_mm, 3.0);
        assert_eq!(saved.trim_short_a_mm, 2.5);
        assert_eq!(saved.trim_long_b_mm, start.trim_long_b_mm);
        assert_eq!(saved.trim_short_b_mm, start.trim_short_b_mm);
    }

    #[test]
    fn a_reading_that_is_not_a_line_fails_and_lists_the_lines() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-typo").join("printer.toml"));
        let (mut term, _) = Terminal::scripted([]);
        let err = run(flag_args(Some(2.2), None, true), &mut term, &file, None).unwrap_err();
        assert_eq!(
            err.to_string(),
            "2.2 mm is not a line on the sheet; use one of 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, \
             5.0, 5.5"
        );
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn flags_without_yes_ask_to_save() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-flags-ask").join("printer.toml"));
        let (mut term, written) = Terminal::scripted([Answer::Confirm(false)]);
        run(flag_args(Some(3.0), None, false), &mut term, &file, None).unwrap();
        assert!(written.out().ends_with("Not saved.\n"), "{}", written.out());
    }

    #[test]
    fn no_terminal_and_no_yes_fails_with_the_hint() {
        let hint = "the prompts need a terminal; give the edges as \
                    --left/--top/--right/--bottom and pass --yes";
        for case in [flag_args(Some(3.0), None, false), read_args()] {
            let file = ConfigFile::at(fresh_dir("cli-calibrate-no-tty").join("printer.toml"));
            let (mut term, _) = Terminal::scripted([]);
            term.is_interactive = false;
            let err = run(case, &mut term, &file, None).unwrap_err();
            assert_eq!(err.to_string(), hint);
            assert!(!file.exists().unwrap());
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
        let code = run(read_args(), &mut term, &file, None).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        let out = written.out();
        assert!(
            out.contains("  left                4.50   3.00  *\n"),
            "{out}"
        );
        assert!(out.contains("  top                 2.10   2.10\n"), "{out}");
        assert!(out.ends_with("Saved.\n"), "{out}");
        assert_eq!(
            file.load(None, None)
                .unwrap()
                .saved_profile()
                .unwrap()
                .trim_long_a_mm,
            3.0
        );
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
            run(read_args(), &mut term, &file, None).unwrap(),
            ExitCode::SUCCESS
        );
        assert!(written.out().ends_with("Not saved.\n"), "{}", written.out());
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn a_cancel_is_a_cancelled_error_and_saves_nothing() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-cancel").join("printer.toml"));
        let (mut term, _) = Terminal::scripted([line(3.0), Answer::Cancel]);
        let err = run(read_args(), &mut term, &file, None).unwrap_err();
        assert!(err.is::<Cancelled>(), "{err:#}");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn no_readings_change_nothing() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-skip").join("printer.toml"));
        let (mut term, written) = Terminal::scripted([Answer::Select(SKIP); 4]);
        assert_eq!(
            run(read_args(), &mut term, &file, None).unwrap(),
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
        run(read_args(), &mut term, &file, None).unwrap();
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

    #[test]
    fn read_for_l_writes_the_l_table_and_keeps_postcard() {
        let file = ConfigFile::at(fresh_dir("cli-calibrate-l").join("printer.toml"));
        let postcard = Profile {
            trim_long_a_mm: 4.0,
            ..Paper::Postcard.starting_profile()
        };
        let before = selphy::config::Config::default().with_profile(Paper::Postcard, postcard);
        file.save(&before).unwrap();
        let answers = [
            line(3.0),
            line(2.0),
            line(3.5),
            line(1.5),
            Answer::Confirm(true),
        ];
        let (mut term, written) = Terminal::scripted(answers);

        run(read_args(), &mut term, &file, Some(Paper::L)).unwrap();

        // The L profile starts with no trims.
        assert!(
            written
                .out()
                .contains("  left                0.00   3.00  *\n"),
            "{}",
            written.out()
        );
        let saved = file.load(None, None).unwrap().saved;
        assert_eq!(saved.postcard, before.postcard);
        assert_eq!(
            saved.l,
            Some(Profile {
                trim_long_a_mm: 3.0,
                trim_short_a_mm: 2.0,
                trim_long_b_mm: 3.5,
                trim_short_b_mm: 1.5,
                ..Paper::L.starting_profile()
            })
        );
        assert_eq!(saved.card, None);
    }

    #[test]
    fn the_sheet_is_drawn_at_the_papers_canvas_and_names_it() {
        let dir = fresh_dir("cli-calibrate-sheet");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let out = dir.join("sheet.jpg");
        let args = CalibrateArgs {
            sheet_only: true,
            read: false,
            out: Some(out.clone()),
            ..read_args()
        };
        let (mut term, written) = Terminal::scripted([]);
        run(args, &mut term, &file, Some(Paper::Card)).unwrap();
        assert!(
            written
                .out()
                .starts_with(&format!("Wrote {} for card paper\n", out.display())),
            "{}",
            written.out()
        );
        let sheet = selphy::imaging::load(&out).unwrap().image;
        // 86 x 54 mm at 300 ppi.
        assert_eq!((sheet.width(), sheet.height()), (1016, 638));
        assert!(!file.exists().unwrap());
    }
}
