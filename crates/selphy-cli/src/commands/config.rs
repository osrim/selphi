//! `selphy config`: shows the printer geometry and its effect on common
//! photo shapes.

use std::io::Write as _;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;

use selphy::config::fields::{CANVAS_LONG, CANVAS_SHORT, Field, MAX_STRETCH};
use selphy::config::{ConfigFile, FIT_ENV, Loaded, Profile};
use selphy::geometry::{self, Edge, Fit, Orientation, Trim};
use selphy::paper::Paper;
use selphy::report::placement_summary;

use crate::terminal::Terminal;

#[derive(Args)]
pub struct ConfigArgs {
    /// Print only the config file's path.
    #[arg(long)]
    path: bool,

    /// Write the paper's values to the config file, for editing by hand.
    #[arg(long, conflicts_with = "path")]
    init: bool,

    /// Show how the photo shapes land with this fit [default: the config's
    /// `fit`, else contain]
    #[arg(long, env = FIT_ENV, value_enum)]
    fit: Option<Fit>,
}

/// Photo shapes shown by `selphy config`, as landscape pixel sizes.
const SAMPLE_SHAPES: [(&str, u32, u32); 4] = [
    ("3:2", 3000, 2000),
    ("4:3", 4000, 3000),
    ("16:9", 1920, 1080),
    ("1:1", 2000, 2000),
];

/// Prints the config file's path, writes the file, or shows the paper's
/// geometry, as the flags select. The geometry is the values this run uses,
/// with each value that an env var overrides marked.
pub fn run(
    args: ConfigArgs,
    term: &mut Terminal,
    file: &ConfigFile,
    paper: Option<Paper>,
) -> Result<ExitCode> {
    let path = file.path().display();
    if args.path {
        writeln!(term.out, "{path}")?;
        return Ok(ExitCode::SUCCESS);
    }
    let exists = file.exists()?;
    let loaded = file.load(paper, args.fit)?;
    if args.init {
        if exists {
            bail!("{path} already exists");
        }
        let saved = loaded.saved_profile()?;
        file.save(&loaded.saved.with_profile(loaded.paper, saved))?;
        writeln!(term.out, "Wrote {path}")?;
        return Ok(ExitCode::SUCCESS);
    }

    show(term, file, exists, &loaded)?;
    Ok(ExitCode::SUCCESS)
}

/// The config header, the paper, the fit, the canvas, the trims and the
/// sample shapes, from the values this run uses.
fn show(term: &mut Terminal, file: &ConfigFile, exists: bool, loaded: &Loaded) -> Result<()> {
    let profile = loaded.profile()?;
    let status = if exists {
        ""
    } else {
        "  (not found: using defaults)"
    };
    writeln!(term.out, "Config  {}{status}", file.path().display())?;
    if !loaded.overrides.is_empty() {
        let set: Vec<String> = loaded
            .overrides
            .iter()
            .map(|o| format!("{}={}", o.field.env, o.value))
            .collect();
        writeln!(term.out, "Env     {}", set.join(", "))?;
    }
    let calibrated: Vec<&str> = Paper::ALL
        .into_iter()
        .filter(|&paper| loaded.saved.profile(paper).is_some())
        .map(Paper::name)
        .collect();
    writeln!(
        term.out,
        "Paper   {} (calibrated: {})",
        loaded.paper,
        calibrated.join(", ")
    )?;
    writeln!(term.out, "Fit     {} ({})", loaded.fit, loaded.fit.label())?;
    writeln!(
        term.out,
        "Canvas  {}{} x {}{} mm, stretch up to {}%{}\n",
        profile.canvas_long_mm,
        from_env(loaded, &CANVAS_LONG),
        profile.canvas_short_mm,
        from_env(loaded, &CANVAS_SHORT),
        profile.max_stretch_pct,
        from_env(loaded, &MAX_STRETCH),
    )?;
    show_trims(term, loaded, &profile)?;

    writeln!(term.out, "\nHow a photo lands")?;
    for (shape, width, height) in SAMPLE_SHAPES {
        let placement = geometry::place(&profile, width, height, loaded.fit)
            .expect("sample sizes are non-zero");
        writeln!(term.out, "  {shape:<6}{}", placement_summary(&placement))?;
    }
    Ok(())
}

/// The trim on each edge in both orientations. A row ends with the env var
/// of each value in it that is overridden, and the column it is in.
fn show_trims(term: &mut Terminal, loaded: &Loaded, profile: &Profile) -> Result<()> {
    writeln!(term.out, "Trim, mm    landscape  portrait")?;
    for edge in Edge::ALL {
        let trim = |orientation| Trim::at(orientation, edge);
        let marks: Vec<String> = Orientation::ALL
            .into_iter()
            .filter_map(|orientation| {
                let field = Field::for_trim(trim(orientation));
                loaded.override_of(field)?;
                Some(format!("{} from {}", orientation.name(), field.env))
            })
            .collect();
        let marks = if marks.is_empty() {
            String::new()
        } else {
            format!("  ({})", marks.join(", "))
        };
        writeln!(
            term.out,
            "  {:<10}{:>9.1}{:>10.1}{marks}",
            edge.name(),
            trim(Orientation::Landscape).mm(profile),
            trim(Orientation::Portrait).mm(profile)
        )?;
    }
    Ok(())
}

/// " (from SELPHY_…)" when an env var overrides `field`, else nothing.
fn from_env(loaded: &Loaded, field: &Field) -> String {
    match loaded.override_of(field) {
        Some(_) => format!(" (from {})", field.env),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use selphy::test_util::fresh_dir;

    use super::*;

    const SHOW: ConfigArgs = ConfigArgs {
        path: false,
        init: false,
        fit: None,
    };

    #[test]
    fn with_no_file_it_shows_the_defaults() {
        let file = ConfigFile::at(fresh_dir("cli-config-missing").join("printer.toml"));
        let (mut term, written) = Terminal::scripted([]);
        assert_eq!(
            run(SHOW, &mut term, &file, None).unwrap(),
            ExitCode::SUCCESS
        );
        let out = written.out();
        assert!(
            out.starts_with(&format!(
                "Config  {}  (not found: using defaults)\nPaper   postcard (calibrated: postcard)\n\
                 Fit     contain (Whole photo)\nCanvas  150 x 100 mm, stretch up to 2.5%\n",
                file.path().display()
            )),
            "{out}"
        );
        assert!(
            out.contains("  3:2   stretched 1.9%, edge to edge\n"),
            "{out}"
        );
        assert_eq!(written.err(), "");
    }

    #[test]
    fn init_writes_the_file_once() {
        let file = ConfigFile::at(fresh_dir("cli-config-init").join("printer.toml"));
        let init = ConfigArgs { init: true, ..SHOW };
        let (mut term, written) = Terminal::scripted([]);
        assert_eq!(
            run(init, &mut term, &file, None).unwrap(),
            ExitCode::SUCCESS
        );
        assert_eq!(written.out(), format!("Wrote {}\n", file.path().display()));
        assert!(file.exists().unwrap());

        let again = ConfigArgs { init: true, ..SHOW };
        let err = run(again, &mut term, &file, None).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("{} already exists", file.path().display())
        );
    }

    #[test]
    fn init_saves_the_file_values_not_the_overrides() {
        let file = ConfigFile::at(fresh_dir("cli-config-init-env").join("printer.toml"))
            .with_overrides([("SELPHY_TRIM_LONG_A_MM", "3")]);
        let init = ConfigArgs { init: true, ..SHOW };
        let (mut term, _) = Terminal::scripted([]);
        run(init, &mut term, &file, None).unwrap();
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("\n[postcard]\n"), "{text}");
        assert!(text.contains("trim_long_a_mm = 4.5"), "{text}");
    }

    #[test]
    fn path_prints_the_path() {
        let file = ConfigFile::at("/somewhere/printer.toml");
        let args = ConfigArgs { path: true, ..SHOW };
        let (mut term, written) = Terminal::scripted([]);
        run(args, &mut term, &file, None).unwrap();
        assert_eq!(written.out(), "/somewhere/printer.toml\n");
    }

    #[test]
    fn an_overridden_value_is_marked_with_its_variable() {
        let file =
            ConfigFile::at(fresh_dir("cli-config-env").join("printer.toml")).with_overrides([
                ("SELPHY_TRIM_LONG_A_MM", "3"),
                ("SELPHY_MAX_STRETCH_PCT", "0"),
            ]);
        let (mut term, written) = Terminal::scripted([]);
        run(SHOW, &mut term, &file, None).unwrap();
        let out = written.out();
        assert!(
            out.contains("\nEnv     SELPHY_TRIM_LONG_A_MM=3, SELPHY_MAX_STRETCH_PCT=0\n"),
            "{out}"
        );
        assert!(
            out.contains("Canvas  150 x 100 mm, stretch up to 0% (from SELPHY_MAX_STRETCH_PCT)\n"),
            "{out}"
        );
        // Long A is the landscape left and the portrait top.
        assert!(
            out.contains(
                "  left            3.0       2.7  (landscape from SELPHY_TRIM_LONG_A_MM)\n"
            ),
            "{out}"
        );
        assert!(
            out.contains(
                "  top             2.1       3.0  (portrait from SELPHY_TRIM_LONG_A_MM)\n"
            ),
            "{out}"
        );
        assert!(out.contains("  3:2   stretched 0.0%"), "{out}");
    }

    #[test]
    fn paper_shows_that_papers_profile_and_lists_the_calibrated_papers() {
        let file = ConfigFile::at(fresh_dir("cli-config-paper").join("printer.toml"));
        fs::write(
            file.path(),
            "[l]\ncanvas_long_mm = 121.0\ntrim_long_a_mm = 1.5\ntrim_long_b_mm = 0.0\n\
             trim_short_a_mm = 0.0\ntrim_short_b_mm = 0.0\n",
        )
        .unwrap();
        let (mut term, written) = Terminal::scripted([]);
        run(SHOW, &mut term, &file, Some(Paper::L)).unwrap();
        let out = written.out();
        assert!(
            out.contains("\nPaper   l (calibrated: postcard, l)\nFit     contain (Whole photo)\nCanvas  121 x 89 mm, "),
            "{out}"
        );
        assert!(out.contains("  left            1.5       0.0\n"), "{out}");
    }

    #[test]
    fn fit_cover_shows_the_cut_edges_of_the_samples() {
        let file = ConfigFile::at(fresh_dir("cli-config-cover").join("printer.toml"));
        let (mut term, written) = Terminal::scripted([]);
        let args = ConfigArgs {
            fit: Some(Fit::Cover),
            ..SHOW
        };
        run(args, &mut term, &file, None).unwrap();
        let out = written.out();
        assert!(out.contains("\nFit     cover (Fill card)\n"), "{out}");
        for shape in ["4:3", "16:9"] {
            let line = out
                .lines()
                .find(|line| line.starts_with(&format!("  {shape} ")))
                .unwrap_or_else(|| panic!("no {shape} line: {out}"));
            assert!(line.contains(", cut "), "{line}");
            assert!(!line.contains("white"), "{line}");
        }
    }

    #[test]
    fn an_uncalibrated_paper_fails_with_the_calibrate_hint() {
        let file = ConfigFile::at(fresh_dir("cli-config-uncalibrated").join("printer.toml"));
        let (mut term, _) = Terminal::scripted([]);
        let err = run(SHOW, &mut term, &file, Some(Paper::Card)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "card paper is not calibrated. Run: selphy calibrate --paper card"
        );
    }
}
