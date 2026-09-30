//! Text formatting shared by the commands.

use selphy::geometry::{Edge, Placement};

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
