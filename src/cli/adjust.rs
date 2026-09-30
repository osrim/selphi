//! `selphy adjust`: turns what a printed card shows on each edge into
//! corrected trims.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use inquire::validator::Validation;
use inquire::{Confirm, CustomType};

use selphy::adjust;
use selphy::config::{self, Config};
use selphy::geometry::{Edge, px_to_mm};
use selphy::record::Record;

use super::report::{answer, print_changes};

#[derive(Args)]
pub struct AdjustArgs {
    /// A JPEG written by `selphy prepare`, printed Borderless.
    file: PathBuf,
}

pub fn run(args: AdjustArgs) -> Result<ExitCode> {
    let record = Record::read(&args.file)?;
    let path = config::default_path();
    let cfg = Config::load(&path)?;

    let margins: Vec<String> = Edge::ALL
        .into_iter()
        .map(|edge| format!("{} {:.2}", edge.name(), px_to_mm(record.margin_px(edge))))
        .collect();
    println!(
        "{}: {}, picture margins {} mm",
        args.file.display(),
        record.orientation.name(),
        margins.join(", ")
    );
    println!("Hold the card {} as printed.\n", record.orientation.name());

    let measured = ask_measurements()?;
    let updated = adjust::apply_measurements(&cfg, &record, &measured)?;
    print_changes(&cfg, &updated, record.orientation);

    let save = Confirm::new(&format!("Save to {}?", path.display()))
        .with_default(true)
        .prompt();
    if answer(save)? {
        updated.save(&path)?;
        println!("Saved.");
    } else {
        println!("Not saved.");
    }
    Ok(ExitCode::SUCCESS)
}

fn ask_measurements() -> Result<Vec<(Edge, f64)>> {
    let help = "positive = white showed, negative = picture was cut, 0 = picture reached the edge";
    let mut measured = Vec::new();
    for edge in Edge::ALL {
        let message = format!("{} edge, mm:", edge.name());
        let mm = CustomType::<f64>::new(&message)
            .with_default(0.0)
            .with_help_message(help)
            .with_error_message("type a number of mm, such as 1.5 or -0.5")
            .with_validator(|mm: &f64| {
                Ok(if mm.is_finite() {
                    Validation::Valid
                } else {
                    Validation::Invalid("type a number of mm, such as 1.5 or -0.5".into())
                })
            })
            .prompt();
        measured.push((edge, answer(mm)?));
    }
    Ok(measured)
}
