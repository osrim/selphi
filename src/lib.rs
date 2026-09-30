//! Prepares photos for borderless printing on a Canon SELPHY, so the printer's
//! edge trim never crops the picture.

#![warn(missing_docs)]

pub mod adjust;
mod atomic;
pub mod calibrate;
pub mod config;
pub mod geometry;
pub mod imaging;
pub mod prepare;
pub mod record;

#[cfg(test)]
mod test_util;
