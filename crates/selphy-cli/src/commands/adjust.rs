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

use crate::terminal::{Terminal, confirm_save, print_changes};

#[derive(Args)]
pub struct AdjustArgs {
    /// A JPEG written by `selphy prepare`, printed Borderless.
    file: PathBuf,
}

/// Asks for the white on each edge of the printed file, then offers to save
/// the corrected trims. They are computed from the record's margins, so
/// starting from the file's values is right even when the photo was
/// prepared with an env override.
pub fn run(args: AdjustArgs, term: &mut Terminal, file: &ConfigFile) -> Result<ExitCode> {
    let record = Record::read(&args.file)?;
    let loaded = file.load()?;

    let margins: Vec<String> = Edge::ALL
        .into_iter()
        .map(|edge| format!("{} {:.2}", edge.name(), px_to_mm(record.margin_px(edge))))
        .collect();
    writeln!(
        term.out,
        "{}: {}, picture margins {} mm",
        args.file.display(),
        record.orientation.name(),
        margins.join(", ")
    )?;
    writeln!(
        term.out,
        "Hold the card {} as printed.\n",
        record.orientation.name()
    )?;

    let measured = ask_measurements(term)?;
    let updated = adjust::apply_measurements(&loaded.saved, &record, &measured)?;
    print_changes(term, &loaded.saved, &updated, record.orientation)?;

    confirm_save(term, &loaded, &updated, file)?;
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

    use selphy::config::Config;
    use selphy::prepare::{Job, Options};
    use selphy::test_util::{fresh_dir, write_jpeg};

    use super::*;
    use crate::terminal::Answer;

    /// A 16:9 photo prepared with the default config, in `dir`. It is
    /// landscape, with white at the top and bottom.
    fn prepared(dir: &Path) -> PathBuf {
        let source = write_jpeg(&dir.join("wide.jpg"), 320, 180, &[]);
        let opts = Options {
            out_dir: dir.join("out"),
            archive_dir: None,
            camera_ref: None,
        };
        let done = Job::new(Config::default(), opts)
            .unwrap()
            .run_one(&source)
            .unwrap();
        done.prepared.output
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
        let code = run(
            AdjustArgs {
                file: photo.clone(),
            },
            &mut term,
            &file,
        )
        .unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        let out = written.out();
        assert!(
            out.starts_with(&format!(
                "{}: landscape, picture margins left 4.49, ",
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
        let saved = file.load().unwrap().saved;
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
        let err = run(
            AdjustArgs {
                file: prepared(&dir),
            },
            &mut term,
            &file,
        )
        .unwrap_err();
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

        run(
            AdjustArgs {
                file: prepared(&dir),
            },
            &mut term,
            &file,
        )
        .unwrap();

        assert_eq!(
            written.err(),
            "SELPHY_TRIM_LONG_A_MM is set; the saved value is not used while it is\n"
        );
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 3.99"), "{text}");
        assert!(text.contains("trim_long_b_mm = 5.5"), "{text}");
    }
}
