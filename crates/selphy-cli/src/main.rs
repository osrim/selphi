//! The `selphy` command line. All the work happens in the library; the
//! `commands` modules parse arguments and report results, one module per
//! command. The stream and exit-code rules are in [`terminal`].

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

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
    #[command(after_help = PREPARE_EXAMPLES)]
    Prepare(commands::prepare::PrepareArgs),
    /// Show the printer geometry and how common photo shapes land on the card.
    #[command(after_help = CONFIG_EXAMPLES)]
    Config(commands::config::ConfigArgs),
    /// Measure the printer's trim: print a bracket sheet, then enter what survived.
    #[command(after_help = CALIBRATE_EXAMPLES)]
    Calibrate(commands::calibrate::CalibrateArgs),
    /// Correct the trims from a printed photo: enter the white or loss on each edge.
    #[command(after_help = ADJUST_EXAMPLES)]
    Adjust(commands::adjust::AdjustArgs),
    /// Print the shell completion script for SHELL to stdout.
    #[command(after_help = COMPLETIONS_EXAMPLES)]
    Completions {
        /// The shell to complete in.
        shell: Shell,
    },
}

const PREPARE_EXAMPLES: &str = "\
Examples:
  selphy prepare                            src/ to out/, sources moved to originals/
  selphy prepare trip/ extra.jpg -o prints  named photos to prints/, left in place
  selphy prepare --fit cover                fill the card, cutting what does not fit
  selphy prepare --dry-run                  show the plan; write and move nothing";

const CONFIG_EXAMPLES: &str = "\
Examples:
  selphy config --init                      write the values to printer.toml
  $EDITOR \"$(selphy config --path)\"         edit it by hand";

const CALIBRATE_EXAMPLES: &str = "\
Examples:
  selphy calibrate                          write the sheet; after printing, enter the readings
  selphy calibrate --read                   enter the readings from a sheet printed earlier
  selphy calibrate --read --left 2.5 --top 2.0 --right 5.5 --bottom 3.0 --yes";

const ADJUST_EXAMPLES: &str = "\
Examples:
  selphy adjust out/a-selphy.jpg            enter the white on each edge
  selphy adjust out/a-selphy.jpg --left 1.0 --bottom -0.5 --yes";

const COMPLETIONS_EXAMPLES: &str = "\
Examples:
  selphy completions zsh > ~/.zfunc/_selphy
  selphy completions bash > ~/.local/share/bash-completion/completions/selphy";

fn main() -> ExitCode {
    let cli = Cli::parse();
    let file = ConfigFile::locate(cli.config);
    let mut term = Terminal::real();
    let result = match cli.command {
        Command::Prepare(args) => commands::prepare::run(args, &mut term, &file),
        Command::Config(args) => commands::config::run(args, &mut term, &file),
        Command::Calibrate(args) => commands::calibrate::run(args, &mut term, &file),
        Command::Adjust(args) => commands::adjust::run(args, &mut term, &file),
        Command::Completions { shell } => completions(shell, &mut term.out),
    };
    finish(result, &mut term.err)
}

/// Writes the completion script for `shell` to `out`.
fn completions(shell: Shell, out: &mut dyn Write) -> Result<ExitCode> {
    clap_complete::generate(shell, &mut Cli::command(), "selphy", out);
    Ok(ExitCode::SUCCESS)
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
    fn the_cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn completions_for_zsh_name_the_commands() {
        let mut out = Vec::new();
        assert_eq!(
            completions(Shell::Zsh, &mut out).unwrap(),
            ExitCode::SUCCESS
        );
        let script = String::from_utf8(out).unwrap();
        assert!(script.contains("prepare"), "{script}");
    }

    #[test]
    fn calibrate_edge_flags_need_read_and_adjust_takes_negative_numbers() {
        let parse = |args: &[&str]| Cli::try_parse_from(args).map(|_| ());
        assert!(parse(&["selphy", "calibrate", "--left", "2.5"]).is_err());
        assert!(parse(&["selphy", "calibrate", "--read", "--left", "2.5", "--yes"]).is_ok());
        assert!(parse(&["selphy", "adjust", "a.jpg", "--bottom", "-0.5", "--yes"]).is_ok());
        assert!(parse(&["selphy", "prepare", "-j", "0"]).is_err());
        assert!(parse(&["selphy", "prepare", "-j", "2", "--dry-run"]).is_ok());
    }

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
