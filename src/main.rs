//! The `selphy` command line. All the work happens in the library; the `cli`
//! modules parse arguments and report results, one module per command.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod cli;

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
    Prepare(cli::prepare::PrepareArgs),
    /// Show the printer geometry and how common photo shapes land on the card.
    Config(cli::config::ConfigArgs),
    /// Measure the printer's trim: print a bracket sheet, then enter what survived.
    Calibrate(cli::calibrate::CalibrateArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Prepare(args) => cli::prepare::run(args),
        Command::Config(args) => cli::config::run(args),
        Command::Calibrate(args) => cli::calibrate::run(args),
    };
    result.unwrap_or_else(|err| {
        eprintln!("error: {err:#}");
        ExitCode::FAILURE
    })
}
