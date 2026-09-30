//! One module per subcommand.

pub mod adjust;
pub mod calibrate;
pub mod config;
pub mod prepare;

use selphy::geometry::Edge;

/// The edges given as `--left`, `--top`, `--right` and `--bottom`, from
/// their values in that order, with the edges not given left out.
fn given_edges(values: [Option<f64>; 4]) -> Vec<(Edge, f64)> {
    Edge::ALL
        .into_iter()
        .zip(values)
        .filter_map(|(edge, mm)| Some((edge, mm?)))
        .collect()
}
