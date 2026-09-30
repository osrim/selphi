//! `selphy config`: shows the printer geometry and its effect on common
//! photo shapes.

use std::io::Write as _;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;

use selphy::config::fields::{CANVAS_LONG, CANVAS_SHORT, Field, MAX_STRETCH};
use selphy::config::{ConfigFile, Loaded};
use selphy::geometry::{self, Edge, Orientation, Trim};
use selphy::report::placement_summary;

use crate::terminal::Terminal;

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
/// the flags select. The geometry is the values this run uses, with each
/// value that an env var overrides marked.
pub fn run(args: ConfigArgs, term: &mut Terminal, file: &ConfigFile) -> Result<ExitCode> {
    let path = file.path().display();
    if args.path {
        writeln!(term.out, "{path}")?;
        return Ok(ExitCode::SUCCESS);
    }
    let exists = file.exists()?;
    let loaded = file.load()?;
    if args.init {
        if exists {
            bail!("{path} already exists");
        }
        file.save(&loaded.saved)?;
        writeln!(term.out, "Wrote {path}")?;
        return Ok(ExitCode::SUCCESS);
    }

    show(term, file, exists, &loaded)?;
    Ok(ExitCode::SUCCESS)
}

/// The config header, the canvas, the trims and the sample shapes, from the
/// values this run uses.
fn show(term: &mut Terminal, file: &ConfigFile, exists: bool, loaded: &Loaded) -> Result<()> {
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
    let cfg = &loaded.effective;
    writeln!(
        term.out,
        "Canvas  {}{} x {}{} mm, stretch up to {}%{}\n",
        cfg.canvas_long_mm,
        from_env(loaded, &CANVAS_LONG),
        cfg.canvas_short_mm,
        from_env(loaded, &CANVAS_SHORT),
        cfg.max_stretch_pct,
        from_env(loaded, &MAX_STRETCH),
    )?;
    show_trims(term, loaded)?;

    writeln!(term.out, "\nHow a photo lands")?;
    for (shape, width, height) in SAMPLE_SHAPES {
        let placement = geometry::place(cfg, width, height).expect("sample sizes are non-zero");
        writeln!(term.out, "  {shape:<6}{}", placement_summary(&placement))?;
    }
    Ok(())
}

/// The trim on each edge in both orientations. A row ends with the env var
/// of each value in it that is overridden, and the column it is in.
fn show_trims(term: &mut Terminal, loaded: &Loaded) -> Result<()> {
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
            trim(Orientation::Landscape).mm(&loaded.effective),
            trim(Orientation::Portrait).mm(&loaded.effective)
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
    };

    #[test]
    fn with_no_file_it_shows_the_defaults() {
        let file = ConfigFile::at(fresh_dir("cli-config-missing").join("printer.toml"));
        let (mut term, written) = Terminal::scripted([]);
        assert_eq!(run(SHOW, &mut term, &file).unwrap(), ExitCode::SUCCESS);
        let out = written.out();
        assert!(
            out.starts_with(&format!(
                "Config  {}  (not found: using defaults)\nCanvas  150 x 100 mm, stretch up to 2.5%\n",
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
        let init = ConfigArgs {
            path: false,
            init: true,
        };
        let (mut term, written) = Terminal::scripted([]);
        assert_eq!(run(init, &mut term, &file).unwrap(), ExitCode::SUCCESS);
        assert_eq!(written.out(), format!("Wrote {}\n", file.path().display()));
        assert!(file.exists().unwrap());

        let again = ConfigArgs {
            path: false,
            init: true,
        };
        let err = run(again, &mut term, &file).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("{} already exists", file.path().display())
        );
    }

    #[test]
    fn init_saves_the_file_values_not_the_overrides() {
        let file = ConfigFile::at(fresh_dir("cli-config-init-env").join("printer.toml"))
            .with_overrides([("SELPHY_TRIM_LONG_A_MM", "3")]);
        let init = ConfigArgs {
            path: false,
            init: true,
        };
        let (mut term, _) = Terminal::scripted([]);
        run(init, &mut term, &file).unwrap();
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 4.5"), "{text}");
    }

    #[test]
    fn path_prints_the_path() {
        let file = ConfigFile::at("/somewhere/printer.toml");
        let args = ConfigArgs {
            path: true,
            init: false,
        };
        let (mut term, written) = Terminal::scripted([]);
        run(args, &mut term, &file).unwrap();
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
        run(SHOW, &mut term, &file).unwrap();
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
}
