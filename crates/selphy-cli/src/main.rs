//! The `selphy` command line. All the work happens in the library; the
//! `commands` modules parse arguments and report results, one module per
//! command.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod commands;
mod terminal;

#[derive(Parser)]
#[command(
    version,
    about = "Prepare photos for borderless printing on a Canon SELPHY"
)]
struct Cli {
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
    let result = match cli.command {
        Command::Prepare(args) => commands::prepare::run(args),
        Command::Config(args) => commands::config::run(args),
        Command::Calibrate(args) => commands::calibrate::run(args),
        Command::Adjust(args) => commands::adjust::run(args),
    };
    result.unwrap_or_else(|err| {
        eprintln!("error: {err:#}");
        ExitCode::FAILURE
    })
}
