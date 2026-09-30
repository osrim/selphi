//! Prompt handling and change reports shared by the commands.

use std::path::Path;

use anyhow::{Result, bail};
use inquire::{Confirm, InquireError};

use selphy::config::Config;
use selphy::geometry::{Edge, Orientation, Trim};

/// The trims on each edge before and after, with changed ones marked.
pub fn print_changes(before: &Config, after: &Config, orientation: Orientation) {
    println!("\nTrim, mm ({})   before  after", orientation.name());
    for edge in Edge::ALL {
        let trim = Trim::at(orientation, edge);
        let (old, new) = (trim.mm(before), trim.mm(after));
        let mark = if old == new { "" } else { "  *" };
        println!("  {:<18}{old:>6.2}{new:>7.2}{mark}", edge.name());
    }
}

/// Asks whether to save `updated` to `path`, and saves it on yes.
pub fn confirm_save(updated: &Config, path: &Path) -> Result<()> {
    let save = Confirm::new(&format!("Save to {}?", path.display()))
        .with_default(true)
        .prompt();
    if answer(save)? {
        updated.save(path)?;
        println!("Saved.");
    } else {
        println!("Not saved.");
    }
    Ok(())
}

/// A prompt's answer, with Esc, Ctrl-C and a missing terminal turned into
/// plain errors.
pub fn answer<T>(result: Result<T, InquireError>) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            bail!("cancelled; nothing saved")
        }
        Err(InquireError::NotTTY) => bail!("the prompts need a terminal"),
        Err(err) => Err(err.into()),
    }
}
