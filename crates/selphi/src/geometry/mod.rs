//! Pure layout maths: where a picture goes on the canvas so that the
//! printer's trim never reaches it. No I/O, so everything here is testable.

mod canvas;
mod placement;

pub use canvas::{Canvas, Edge, Orientation, PPI, Trim, mm_to_px, px_to_mm};
pub use placement::{Fit, Placement, place};
