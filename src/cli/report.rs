//! Text formatting and prompt handling shared by the commands.

use anyhow::{Result, bail};
use inquire::InquireError;

use selphy::config::Config;
use selphy::geometry::{Edge, Orientation, Placement, Trim};

/// "edge to edge", or which edges keep white margin and how much.
pub fn white_summary(placement: &Placement) -> String {
    let white: Vec<String> = Edge::ALL
        .into_iter()
        .map(|edge| (edge, placement.white_mm(edge)))
        .filter(|&(_, mm)| mm >= 0.05)
        .map(|(edge, mm)| format!("{} {mm:.1}", edge.name()))
        .collect();
    if white.is_empty() {
        "edge to edge".to_string()
    } else {
        format!("white {} mm", white.join(", "))
    }
}

/// The error and its causes, joined with ": ". A cause is skipped when the
/// message before it already quotes it, as the `image` crate's errors do.
pub fn error_chain(err: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in err.chain() {
        let text = cause.to_string();
        if !parts.last().is_some_and(|prev| prev.contains(&text)) {
            parts.push(text);
        }
    }
    parts.join(": ")
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context, anyhow};
    use selphy::config::Config;

    #[test]
    fn error_chain_drops_causes_already_quoted() {
        let err = Err::<(), _>(anyhow!("bad bytes"))
            .context("Format error: bad bytes")
            .context("reading a.jpg")
            .unwrap_err();
        assert_eq!(error_chain(&err), "reading a.jpg: Format error: bad bytes");
    }

    #[test]
    fn white_summary_names_only_edges_with_white() {
        let cfg = Config::default();
        let fill = selphy::geometry::place(&cfg, 3616, 5424).unwrap();
        assert_eq!(white_summary(&fill), "edge to edge");
        let wide = selphy::geometry::place(&cfg, 1920, 1080).unwrap();
        assert_eq!(white_summary(&wide), "white top 7.2, bottom 7.2 mm");
    }
}
