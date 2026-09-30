//! The `selphy` command line. All the work happens in the library; the
//! `commands` modules parse arguments and report results, one module per
//! command. The stream and exit-code rules are in [`terminal`].

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use selphy::config::ConfigFile;

use terminal::{Cancelled, Terminal};

mod commands;
mod terminal;

#[derive(Parser)]
#[command(
    version,
    about = "Prepare photos for borderless printing on a Canon SELPHY"
)]
struct Cli {
    /// The printer config file [default: ~/.config/selphy/printer.toml]
    #[arg(long, global = true, env = "SELPHY_CONFIG", value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Turn photos into print-ready JPEGs that the printer's trim never crops.
    Prepare(commands::prepare::PrepareArgs),
    /// Show the printer geometry and how common photo shapes land on the card.
    Config(commands::config::ConfigArgs),
    /// Measure the printer's trim: print a bracket sheet, then enter what survived.
    Calibrate(commands::calibrate::CalibrateArgs),
    /// Correct the trims from a printed photo: enter the white or loss on each edge.
    Adjust(commands::adjust::AdjustArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let file = ConfigFile::locate(cli.config);
    let mut term = Terminal::real();
    let result = match cli.command {
        Command::Prepare(args) => commands::prepare::run(args, &mut term, &file),
        Command::Config(args) => commands::config::run(args, &mut term, &file),
        Command::Calibrate(args) => commands::calibrate::run(args, &mut term, &file),
        Command::Adjust(args) => commands::adjust::run(args, &mut term, &file),
    };
    finish(result, &mut term.err)
}

/// The exit code for a command's result. An error is printed to `err`, and
/// gives 130 when it was a cancelled prompt, else 1.
fn finish(result: Result<ExitCode>, err: &mut dyn Write) -> ExitCode {
    result.unwrap_or_else(|error| {
        let _ = writeln!(err, "error: {error:#}");
        if error.is::<Cancelled>() {
            ExitCode::from(130)
        } else {
            ExitCode::FAILURE
        }
    })
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;

    #[test]
    fn an_error_is_printed_and_exits_1_and_a_cancel_exits_130() {
        let mut err = Vec::new();
        assert_eq!(finish(Ok(ExitCode::SUCCESS), &mut err), ExitCode::SUCCESS);
        assert!(err.is_empty());

        let code = finish(Err(anyhow!("inner").context("outer")), &mut err);
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(
            String::from_utf8(err.clone()).unwrap(),
            "error: outer: inner\n"
        );

        err.clear();
        let code = finish(Err(Cancelled.into()), &mut err);
        assert_eq!(code, ExitCode::from(130));
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "error: cancelled; nothing saved\n"
        );
    }

    #[test]
    fn config_works_before_and_after_the_command_and_wins_over_the_env() {
        let parse = |args: &[&str]| Cli::try_parse_from(args).unwrap().config;
        assert_eq!(
            parse(&["selphy", "--config", "a.toml", "config"]),
            Some(PathBuf::from("a.toml"))
        );
        assert_eq!(
            parse(&["selphy", "config", "--config", "a.toml"]),
            Some(PathBuf::from("a.toml"))
        );
        // SAFETY: the only test in this process that reads or writes
        // SELPHY_CONFIG.
        unsafe { std::env::set_var("SELPHY_CONFIG", "env.toml") };
        let from_env = parse(&["selphy", "config"]);
        let from_flag = parse(&["selphy", "config", "--config", "a.toml"]);
        // SAFETY: as above.
        unsafe { std::env::remove_var("SELPHY_CONFIG") };
        assert_eq!(from_env, Some(PathBuf::from("env.toml")));
        assert_eq!(from_flag, Some(PathBuf::from("a.toml")));
    }
}
