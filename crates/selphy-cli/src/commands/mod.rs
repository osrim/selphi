pub mod adjust;
pub mod calibrate;
pub mod config;
pub mod prepare;

use anyhow::{Result, bail};
use clap::Args;

use selphy::geometry::Edge;

use crate::terminal::Terminal;

/// The measurements of a printed card as flags, and `--yes`, so that
/// `calibrate --read` and `adjust` can run without prompts.
#[derive(Args)]
pub struct EdgeArgs {
    /// The number for the left edge, in mm. With any edge flag, no edge is
    /// asked for, and the edges not given keep their trim.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    left: Option<f64>,

    /// The number for the top edge, in mm.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    top: Option<f64>,

    /// The number for the right edge, in mm.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    right: Option<f64>,

    /// The number for the bottom edge, in mm.
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    bottom: Option<f64>,

    /// Save the trims without asking.
    #[arg(long)]
    pub yes: bool,
}

impl EdgeArgs {
    /// The edges given as flags, or, when none is, the answers from `ask`.
    /// Fails with a hint that names the flags when a prompt is needed, for
    /// the edges or for the save without `--yes`, and there is no terminal.
    fn edges_or_ask(
        &self,
        term: &mut Terminal,
        ask: impl FnOnce(&mut Terminal) -> Result<Vec<(Edge, f64)>>,
    ) -> Result<Vec<(Edge, f64)>> {
        let given = self.given();
        if (given.is_empty() || !self.yes) && !term.is_interactive {
            bail!(
                "the prompts need a terminal; give the edges as --left/--top/--right/--bottom \
                 and pass --yes"
            );
        }
        if given.is_empty() {
            ask(term)
        } else {
            Ok(given)
        }
    }

    fn given(&self) -> Vec<(Edge, f64)> {
        [
            (Edge::Left, self.left),
            (Edge::Top, self.top),
            (Edge::Right, self.right),
            (Edge::Bottom, self.bottom),
        ]
        .into_iter()
        .filter_map(|(edge, mm)| Some((edge, mm?)))
        .collect()
    }

    /// No edge flags and no `--yes`: the prompts ask for everything.
    #[cfg(test)]
    fn none() -> EdgeArgs {
        EdgeArgs {
            left: None,
            top: None,
            right: None,
            bottom: None,
            yes: false,
        }
    }
}
