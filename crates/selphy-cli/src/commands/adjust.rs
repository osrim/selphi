//! `selphy adjust`: turns what a printed card shows on each edge into
//! corrected trims.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;

use selphy::adjust;
use selphy::config::ConfigFile;
use selphy::geometry::{Edge, px_to_mm};
use selphy::paper::Paper;
use selphy::record::Record;

use crate::commands::given_edges;
use crate::terminal::{Terminal, confirm_save, print_changes};

#[derive(Args)]
pub struct AdjustArgs {
    /// A JPEG written by `selphy prepare`, printed Borderless.
    file: PathBuf,

    /// The white on the left edge of the card, or the picture lost there as
    /// a negative number. With any edge flag, the edges are not asked for,
    /// and the edges not given keep their trim.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    left: Option<f64>,

    /// The white on the top edge, or the picture lost there.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    top: Option<f64>,

    /// The white on the right edge, or the picture lost there.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    right: Option<f64>,

    /// The white on the bottom edge, or the picture lost there.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    bottom: Option<f64>,

    /// Save the trims without asking.
    #[arg(long)]
    yes: bool,
}

/// Asks for the white on each edge of the printed file, then offers to save
/// the corrected trims to the profile of the paper the file was prepared
/// for. `paper`, from `--paper` or `SELPHY_PAPER`, is ignored, with a
/// warning when it differs. The trims are
/// computed from the record's margins, so starting from the file's values is
/// right even when the photo was prepared with an env override.
///
/// A file that cannot correct the profile, such as a Fill card print, is an
/// error before the prompts. Measurements given as edge flags are not asked
/// for, and `--yes` saves without asking.
pub fn run(
    args: AdjustArgs,
    term: &mut Terminal,
    file: &ConfigFile,
    paper: Option<Paper>,
) -> Result<ExitCode> {
    let record = Record::read(&args.file)?;
    if let Some(paper) = paper.filter(|&paper| paper != record.paper) {
        writeln!(
            term.err,
            "the {paper} paper setting is ignored: {} was prepared for {} paper",
            args.file.display(),
            record.paper
        )?;
    }
    // The record names the paper and the fit, so the env and the file do not.
    let loaded = file.load(Some(record.paper), Some(record.fit))?;
    let before = loaded.saved_profile()?;
    adjust::check_record(&before, &record)?;

    let margins: Vec<String> = Edge::ALL
        .into_iter()
        .map(|edge| format!("{} {:.2}", edge.name(), px_to_mm(record.margin_px(edge))))
        .collect();
    writeln!(
        term.out,
        "{}: {} paper, {}, picture margins {} mm",
        args.file.display(),
        record.paper,
        record.orientation.name(),
        margins.join(", ")
    )?;
    writeln!(
        term.out,
        "Hold the card {} as printed.\n",
        record.orientation.name()
    )?;

    let given = given_edges([args.left, args.top, args.right, args.bottom]);
    let measured = if given.is_empty() {
        term.require_prompts()?;
        ask_measurements(term)?
    } else {
        given
    };
    let updated = adjust::apply_measurements(&before, &record, &measured)?;
    print_changes(term, &before, &updated, record.orientation)?;

    confirm_save(term, &loaded, (&before, &updated), file, args.yes)?;
    Ok(ExitCode::SUCCESS)
}

fn ask_measurements(term: &mut Terminal) -> Result<Vec<(Edge, f64)>> {
    let help = "positive = white showed, negative = picture was cut, 0 = picture reached the edge";
    let mut measured = Vec::new();
    for edge in Edge::ALL {
        let message = format!("{} edge, mm:", edge.name());
        measured.push((edge, term.number(&message, 0.0, help)?));
    }
    Ok(measured)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use selphy::config::{Config, Profile};
    use selphy::geometry::Fit;
    use selphy::prepare::{Job, Options};
    use selphy::test_util::{fresh_dir, write_jpeg};

    use super::*;
    use crate::terminal::Answer;

    /// A 16:9 photo prepared with the postcard defaults, in `dir`. It is
    /// landscape, with white at the top and bottom.
    fn prepared(dir: &Path) -> PathBuf {
        prepared_for(dir, Paper::Postcard, Paper::Postcard.starting_profile())
    }

    /// A 16:9 photo prepared for `paper` with `profile`, in `dir`.
    fn prepared_for(dir: &Path, paper: Paper, profile: Profile) -> PathBuf {
        prepared_with(dir, paper, profile, Fit::Contain)
    }

    /// A 16:9 photo prepared for `paper` with `profile` and `fit`, in `dir`.
    fn prepared_with(dir: &Path, paper: Paper, profile: Profile, fit: Fit) -> PathBuf {
        let source = write_jpeg(&dir.join("wide.jpg"), 320, 180, &[]);
        let opts = Options {
            out_dir: dir.join("out"),
            archive_dir: None,
            camera_ref: None,
            fit,
        };
        let planned = selphy::prepare::plan(&[source], &opts.out_dir).unwrap();
        let done = Job::new(paper, profile, opts)
            .unwrap()
            .run_one(&planned[0])
            .unwrap();
        done.prepared.output
    }

    /// The args for `file` with no flags.
    fn args(file: PathBuf) -> AdjustArgs {
        AdjustArgs {
            file,
            left: None,
            top: None,
            right: None,
            bottom: None,
            yes: false,
        }
    }

    #[test]
    fn flags_and_yes_save_without_a_prompt_and_keep_the_other_edges() {
        let dir = fresh_dir("cli-adjust-flags");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let (mut term, written) = Terminal::scripted([]);
        let args = AdjustArgs {
            left: Some(0.5),
            right: Some(-0.3),
            yes: true,
            ..args(prepared(&dir))
        };

        let code = run(args, &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(written.out().ends_with("Saved.\n"), "{}", written.out());
        let saved = file.load(None, None).unwrap().saved_profile().unwrap();
        assert_eq!(saved.trim_long_a_mm, 3.99);
        assert_eq!(saved.trim_long_b_mm, 5.8);
        // Top and bottom were not given: they keep their trim, not 0 mm of
        // white.
        assert_eq!(saved.trim_short_a_mm, 2.1);
        assert_eq!(saved.trim_short_b_mm, 2.7);
    }

    #[test]
    fn a_bad_flag_measurement_fails() {
        let dir = fresh_dir("cli-adjust-flags-bad");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let (mut term, _) = Terminal::scripted([]);
        let args = AdjustArgs {
            left: Some(9.0),
            yes: true,
            ..args(prepared(&dir))
        };
        let err = run(args, &mut term, &file, None).unwrap_err();
        assert!(format!("{err:#}").contains("on the left edge"), "{err:#}");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn no_terminal_and_no_yes_fails_with_the_hint() {
        let dir = fresh_dir("cli-adjust-no-tty");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let photo = prepared(&dir);
        let flags = AdjustArgs {
            left: Some(0.5),
            ..args(photo.clone())
        };
        for case in [flags, args(photo)] {
            let (mut term, _) = Terminal::scripted([]);
            term.is_interactive = false;
            let err = run(case, &mut term, &file, None).unwrap_err();
            assert_eq!(
                err.to_string(),
                "the prompts need a terminal; give the edges as --left/--top/--right/--bottom \
                 and pass --yes"
            );
        }
        assert!(!file.exists().unwrap());
    }

    fn numbers(mm: [f64; 4]) -> Vec<Answer> {
        mm.into_iter().map(Answer::Number).collect()
    }

    #[test]
    fn measurements_show_the_changes_and_save_the_trims() {
        let dir = fresh_dir("cli-adjust");
        let file = ConfigFile::at(dir.join("printer.toml"));
        // left, top, right, bottom: 0.5 mm of white on the left, 0.3 mm cut
        // on the right, and the planned white at the top and bottom.
        let mut answers = numbers([0.5, 7.21, -0.3, 7.21]);
        answers.push(Answer::Confirm(true));
        let (mut term, written) = Terminal::scripted(answers);

        let photo = prepared(&dir);
        let code = run(args(photo.clone()), &mut term, &file, None).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        let out = written.out();
        assert!(
            out.starts_with(&format!(
                "{}: postcard paper, landscape, picture margins left 4.49, ",
                photo.display()
            )),
            "{out}"
        );
        assert!(
            out.contains("  left                4.50   3.99  *\n"),
            "{out}"
        );
        assert!(out.contains("  top                 2.10   2.10\n"), "{out}");
        assert!(
            out.contains("  right               5.50   5.80  *\n"),
            "{out}"
        );
        assert!(out.ends_with("Saved.\n"), "{out}");
        let saved = file.load(None, None).unwrap().saved_profile().unwrap();
        assert_eq!(saved.trim_long_a_mm, 3.99);
        assert_eq!(saved.trim_long_b_mm, 5.8);
        assert_eq!(saved.trim_short_a_mm, 2.1);
        assert_eq!(saved.trim_short_b_mm, 2.7);
    }

    #[test]
    fn a_bad_measurement_is_an_error_before_the_save_prompt() {
        let dir = fresh_dir("cli-adjust-bad");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let (mut term, _) = Terminal::scripted(numbers([9.0, 0.0, 0.0, 0.0]));
        let err = run(args(prepared(&dir)), &mut term, &file, None).unwrap_err();
        assert!(format!("{err:#}").contains("on the left edge"), "{err:#}");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn an_override_on_a_measured_edge_saves_the_file_value_and_warns() {
        let dir = fresh_dir("cli-adjust-env");
        let file = ConfigFile::at(dir.join("printer.toml")).with_overrides([
            ("SELPHY_TRIM_LONG_A_MM", "3"),
            ("SELPHY_TRIM_LONG_B_MM", "1"),
        ]);
        let mut answers = numbers([0.5, 7.21, 0.0, 7.21]);
        answers.push(Answer::Confirm(true));
        let (mut term, written) = Terminal::scripted(answers);

        run(args(prepared(&dir)), &mut term, &file, None).unwrap();

        assert_eq!(
            written.err(),
            "SELPHY_TRIM_LONG_A_MM is set; the saved value is not used while it is\n"
        );
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 3.99"), "{text}");
        assert!(text.contains("trim_long_b_mm = 5.5"), "{text}");
    }

    fn l_profile() -> Profile {
        Profile {
            trim_long_a_mm: 2.0,
            trim_long_b_mm: 2.0,
            trim_short_a_mm: 1.0,
            trim_short_b_mm: 1.0,
            ..Paper::L.starting_profile()
        }
    }

    #[test]
    fn an_l_file_corrects_the_l_table_whatever_the_paper_flag_says() {
        let dir = fresh_dir("cli-adjust-l");
        let file = ConfigFile::at(dir.join("printer.toml"));
        file.save(&Config::default().with_profile(Paper::L, l_profile()))
            .unwrap();
        let photo = prepared_for(&dir, Paper::L, l_profile());
        // left 0.5 mm of white; top, right and bottom as planned.
        let (mut term, written) = Terminal::scripted([
            Answer::Number(0.5),
            Answer::Number(6.5),
            Answer::Number(0.0),
            Answer::Number(6.5),
            Answer::Confirm(true),
        ]);

        run(args(photo.clone()), &mut term, &file, Some(Paper::Card)).unwrap();

        assert_eq!(
            written.err(),
            format!(
                "the card paper setting is ignored: {} was prepared for l paper\n",
                photo.display()
            )
        );
        let saved = file.load(None, None).unwrap().saved;
        assert_eq!(saved.postcard, None, "postcard is untouched");
        let l = saved.l.unwrap();
        // The left and right margins are 24 px, 2.03 mm: trim = margin - white.
        assert_eq!(l.trim_long_a_mm, 1.53);
        assert_eq!(l.trim_long_b_mm, 2.03);
    }

    #[test]
    fn a_fill_card_print_is_refused_before_the_prompts() {
        let dir = fresh_dir("cli-adjust-cover");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let photo = prepared_with(
            &dir,
            Paper::Postcard,
            Paper::Postcard.starting_profile(),
            Fit::Cover,
        );
        let (mut term, written) = Terminal::scripted([]);

        let err = run(args(photo), &mut term, &file, None).unwrap_err();

        assert_eq!(
            err.to_string(),
            "a Fill card print cannot be measured; prepare it with the Whole photo fit (--fit \
             contain)"
        );
        assert_eq!(written.out(), "");
    }

    #[test]
    fn a_changed_canvas_is_refused_before_the_prompts() {
        let dir = fresh_dir("cli-adjust-canvas");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let photo = prepared(&dir);
        let smaller = Profile {
            canvas_long_mm: 148.0,
            ..Paper::Postcard.starting_profile()
        };
        file.save(&Config::default().with_profile(Paper::Postcard, smaller))
            .unwrap();
        let (mut term, written) = Terminal::scripted([]);

        let err = run(args(photo), &mut term, &file, None).unwrap_err();

        assert!(
            err.to_string()
                .starts_with("the postcard canvas has changed since this file was prepared"),
            "{err:#}"
        );
        assert_eq!(written.out(), "");
    }
}
