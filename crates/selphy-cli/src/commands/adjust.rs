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
use selphy::record::Record;

use crate::commands::EdgeArgs;
use crate::terminal::{Change, Terminal, confirm_save, print_changes};

#[derive(Args)]
pub struct AdjustArgs {
    /// A JPEG written by `selphy prepare`, printed Borderless.
    file: PathBuf,

    // The measurements: the white on each edge of the card, or the picture
    // lost there as a negative number.
    #[command(flatten, next_help_heading = "Measurements without prompts")]
    edges: EdgeArgs,
}

/// Asks for the white on each edge of the printed file, then offers to save
/// the corrected trims. The trims are computed from the record's margins, so starting from the file's values is
/// right even when the photo was prepared with an env override.
///
/// A file that cannot correct the profile, such as a Fill card print, is an
/// error before the prompts. Measurements given as edge flags are not asked
/// for, and `--yes` saves without asking.
pub fn run(args: AdjustArgs, term: &mut Terminal, file: &ConfigFile) -> Result<ExitCode> {
    let record = Record::read(&args.file)?;
    // The record names the fit, so the env and the file do not.
    let loaded = file.load(Some(record.fit))?;
    let before = loaded.saved_profile();
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

    let measured = args.edges.edges_or_ask(term, ask_measurements)?;
    let change = Change {
        after: adjust::apply_measurements(&before, &record, &measured)?,
        before,
        orientation: record.orientation,
    };
    print_changes(term, &change)?;
    confirm_save(term, &loaded, &change, file, args.edges.yes)?;
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
    use selphy::test_util::{fresh_dir, postcard, write_jpeg};

    use super::*;
    use crate::terminal::Answer;

    /// A 16:9 photo prepared with the defaults, in `dir`. It is landscape,
    /// with white at the top and bottom.
    fn prepared(dir: &Path) -> PathBuf {
        prepared_with(dir, Fit::Contain)
    }

    /// A 16:9 photo prepared with the defaults and `fit`, in `dir`.
    fn prepared_with(dir: &Path, fit: Fit) -> PathBuf {
        let source = write_jpeg(&dir.join("wide.jpg"), 320, 180, &[]);
        let opts = Options {
            out_dir: dir.join("out"),
            archive_dir: None,
            camera_ref: None,
            fit,
        };
        let planned = selphy::prepare::plan(&[source], &opts.out_dir).unwrap();
        let done = Job::new(postcard(), opts)
            .unwrap()
            .run_one(&planned[0])
            .unwrap();
        done.prepared.output
    }

    /// The args for `file` with no flags.
    fn args(file: PathBuf) -> AdjustArgs {
        AdjustArgs {
            file,
            edges: EdgeArgs::none(),
        }
    }

    #[test]
    fn flags_and_yes_save_without_a_prompt_and_keep_the_other_edges() {
        let dir = fresh_dir("cli-adjust-flags");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let (mut term, written) = Terminal::scripted([]);
        let args = AdjustArgs {
            edges: EdgeArgs {
                left: Some(0.5),
                right: Some(-0.3),
                yes: true,
                ..EdgeArgs::none()
            },
            ..args(prepared(&dir))
        };

        let code = run(args, &mut term, &file).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(written.out().ends_with("Saved.\n"), "{}", written.out());
        let saved = file.load(None).unwrap().saved_profile();
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
            edges: EdgeArgs {
                left: Some(9.0),
                yes: true,
                ..EdgeArgs::none()
            },
            ..args(prepared(&dir))
        };
        let err = run(args, &mut term, &file).unwrap_err();
        assert!(format!("{err:#}").contains("on the left edge"), "{err:#}");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn no_terminal_and_no_yes_fails_with_the_hint() {
        let dir = fresh_dir("cli-adjust-no-tty");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let photo = prepared(&dir);
        let flags = AdjustArgs {
            edges: EdgeArgs {
                left: Some(0.5),
                ..EdgeArgs::none()
            },
            ..args(photo.clone())
        };
        for case in [flags, args(photo)] {
            let (mut term, _) = Terminal::scripted([]);
            term.is_interactive = false;
            let err = run(case, &mut term, &file).unwrap_err();
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
        let code = run(args(photo.clone()), &mut term, &file).unwrap();

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
        let saved = file.load(None).unwrap().saved_profile();
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
        let err = run(args(prepared(&dir)), &mut term, &file).unwrap_err();
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

        run(args(prepared(&dir)), &mut term, &file).unwrap();

        assert_eq!(
            written.err(),
            "SELPHY_TRIM_LONG_A_MM is set; the saved value is not used while it is\n"
        );
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 3.99"), "{text}");
        assert!(text.contains("trim_long_b_mm = 5.5"), "{text}");
    }

    #[test]
    fn a_fill_card_print_is_refused_before_the_prompts() {
        let dir = fresh_dir("cli-adjust-cover");
        let file = ConfigFile::at(dir.join("printer.toml"));
        let photo = prepared_with(&dir, Fit::Cover);
        let (mut term, written) = Terminal::scripted([]);

        let err = run(args(photo), &mut term, &file).unwrap_err();

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
            ..postcard()
        };
        file.save(&Config::default().with_profile(smaller)).unwrap();
        let (mut term, written) = Terminal::scripted([]);

        let err = run(args(photo), &mut term, &file).unwrap_err();

        assert!(
            err.to_string()
                .starts_with("the postcard canvas has changed since this file was prepared"),
            "{err:#}"
        );
        assert_eq!(written.out(), "");
    }
}
