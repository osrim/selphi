//! The printer config: the default fit and the postcard [`Profile`].
//! [`ConfigFile`] stores it as TOML at `~/.config/selphy/printer.toml`. A
//! missing file means "use the defaults": the values measured on the first
//! SELPHY CP1500 this ran on.

use std::fmt;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

pub mod fields;
mod file;

pub use file::{ConfigFile, FIT_ENV, Loaded, Override};

use crate::geometry::{Edge, Fit, Orientation, Trim, mm_to_px};
use crate::paper::Paper;

/// The config file: the default fit and the `[postcard]` table. Without a
/// table, postcard uses [`Paper::default_profile`]. Keys missing from the
/// table take their value from it. Unknown keys are an error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawConfig")]
pub struct Config {
    /// The fit to use when none is named on the command line or in
    /// `SELPHY_FIT`. `None` means contain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<Fit>,
    /// The `[postcard]` table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postcard: Option<Profile>,
}

impl Config {
    /// The postcard profile: the table, else the built-in profile.
    pub fn profile(&self) -> Profile {
        self.postcard
            .clone()
            .unwrap_or_else(|| Paper::Postcard.default_profile())
    }

    /// This config with `profile` as the `[postcard]` table.
    pub fn with_profile(&self, profile: Profile) -> Config {
        Config {
            postcard: Some(profile),
            ..self.clone()
        }
    }

    /// Checks the table with [`Profile::validate`]. An error names the
    /// table, as in `[postcard] trim_long_a_mm = -1: must be 0 or more`.
    pub fn validate(&self) -> Result<()> {
        if let Some(profile) = &self.postcard {
            profile
                .validate()
                .map_err(|invalid| anyhow!("[{}] {invalid}", Paper::Postcard))?;
        }
        Ok(())
    }
}

/// The file as written, before the table is filled in from the built-in
/// profile.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    fit: Option<Fit>,
    postcard: Option<toml::Table>,
}

impl TryFrom<RawConfig> for Config {
    type Error = String;

    fn try_from(raw: RawConfig) -> Result<Self, String> {
        let postcard = raw
            .postcard
            .map(fill_in)
            .transpose()
            .map_err(|err| format!("[{}] {err}", Paper::Postcard))?;
        Ok(Config {
            fit: raw.fit,
            postcard,
        })
    }
}

/// The profile in `table`, with each missing key taken from the built-in
/// profile.
fn fill_in(table: toml::Table) -> Result<Profile, String> {
    let mut merged = toml::Table::try_from(Paper::Postcard.default_profile())
        .expect("a profile serialises to a table");
    merged.extend(table);
    toml::Value::Table(merged)
        .try_into()
        .map_err(|err: toml::de::Error| err.to_string())
}

/// The printer geometry for postcard paper: the canvas, the four trims, and
/// the max stretch.
///
/// Trims are millimetres of canvas the printer loses on each edge in
/// Borderless mode. They are named for the LANDSCAPE canvas: long A is the
/// left end, long B the right end, short A the top edge, short B the bottom.
/// `geometry` maps them onto a portrait canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// The canvas handed to the printer. It is bigger than the paper, because
    /// the printer enlarges and trims. The postcard canvas is 150x100mm, NOT
    /// 4x6 inch.
    pub canvas_long_mm: f64,
    /// The canvas's short side.
    pub canvas_short_mm: f64,
    /// Trim on the landscape canvas's left end.
    pub trim_long_a_mm: f64,
    /// Trim on the landscape canvas's right end.
    pub trim_long_b_mm: f64,
    /// Trim on the landscape canvas's top edge.
    pub trim_short_a_mm: f64,
    /// Trim on the landscape canvas's bottom edge.
    pub trim_short_b_mm: f64,
    /// Largest one-axis stretch, in percent, used to close the gap between
    /// the picture's aspect and the card's. 2:3 needs 1.9%.
    pub max_stretch_pct: f64,
}

impl Profile {
    /// This profile with the trim on each edge in `trims` set, mapped
    /// through [`Trim::at`] for `orientation`. Edges not in `trims` keep
    /// their trim. The result is validated, and a trim error names the edges
    /// of `orientation`, not the TOML keys.
    pub fn with_trims(&self, orientation: Orientation, trims: &[(Edge, f64)]) -> Result<Profile> {
        let mut updated = self.clone();
        for &(edge, mm) in trims {
            *Trim::at(orientation, edge).mm_mut(&mut updated) = mm;
        }
        updated
            .validate()
            .map_err(|invalid| anyhow!(invalid.for_edges(orientation)))?;
        Ok(updated)
    }

    /// Checks that the values describe a printable canvas: the canvas sides
    /// are more than 0, the trims and the stretch are 0 or more, and on each
    /// side the two trims leave at least one pixel.
    pub fn validate(&self) -> Result<(), Invalid> {
        for side in Side::ALL {
            let value = side.canvas_mm(self);
            if !(value.is_finite() && value > 0.0) {
                return Err(Invalid::Canvas { side, value });
            }
        }
        for trim in Trim::ALL {
            let value = trim.mm(self);
            if !non_negative(value) {
                return Err(Invalid::NegativeTrim { trim, value });
            }
        }
        let value = self.max_stretch_pct;
        if !non_negative(value) {
            return Err(Invalid::Stretch { value });
        }
        for side in Side::ALL {
            let side_mm = side.canvas_mm(self);
            let [a, b] = side.trims().map(|trim| trim.mm(self));
            if mm_to_px(side_mm) - mm_to_px(a) - mm_to_px(b) < 1 {
                return Err(Invalid::NothingLeft {
                    side,
                    side_mm,
                    sum_mm: a + b,
                });
            }
        }
        Ok(())
    }
}

/// A finite number that is 0 or more.
fn non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// The check that [`Profile::validate`] failed, and the fields it failed on.
/// `Display` names the TOML keys, so that the user can fix the file.
#[derive(Debug, Clone, PartialEq)]
pub enum Invalid {
    /// A canvas side is not more than 0.
    Canvas {
        /// The canvas side.
        side: Side,
        /// Its value, in mm.
        value: f64,
    },
    /// A trim is less than 0, or not a number.
    NegativeTrim {
        /// The trim.
        trim: Trim,
        /// Its value, in mm.
        value: f64,
    },
    /// The max stretch is less than 0, or not a number.
    Stretch {
        /// Its value, in percent.
        value: f64,
    },
    /// The two trims on one canvas side leave no pixel of it.
    NothingLeft {
        /// The canvas side.
        side: Side,
        /// The length of the side, in mm.
        side_mm: f64,
        /// The two trims added, in mm.
        sum_mm: f64,
    },
}

impl Invalid {
    /// The message with the trims named by their edges on an `orientation`
    /// canvas. The canvas and stretch messages keep their TOML keys.
    fn for_edges(&self, orientation: Orientation) -> String {
        match *self {
            Self::NegativeTrim { trim, value } => {
                format!(
                    "the {} trim is {value} mm: it must be 0 or more",
                    trim.edge(orientation).name()
                )
            }
            Self::NothingLeft {
                side,
                side_mm,
                sum_mm,
            } => {
                // In the order of Edge::ALL, so a side reads "left and right".
                let mut edges = side.trims().map(|trim| trim.edge(orientation));
                edges.sort();
                format!(
                    "the {} and {} trims ({sum_mm} mm) leave nothing of the {side_mm} mm side",
                    edges[0].name(),
                    edges[1].name()
                )
            }
            Self::Canvas { .. } | Self::Stretch { .. } => self.to_string(),
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Canvas { side, value } => {
                write!(
                    f,
                    "canvas_{}_mm = {value}: must be more than 0",
                    side.name()
                )
            }
            Self::NegativeTrim { trim, value } => {
                write!(f, "{} = {value}: must be 0 or more", trim.key())
            }
            Self::Stretch { value } => write!(f, "max_stretch_pct = {value}: must be 0 or more"),
            Self::NothingLeft {
                side,
                side_mm,
                sum_mm,
            } => {
                let side = side.name();
                write!(
                    f,
                    "trim_{side}_a_mm + trim_{side}_b_mm = {sum_mm} mm leaves nothing of the \
                     {side_mm} mm canvas_{side}_mm"
                )
            }
        }
    }
}

impl std::error::Error for Invalid {}

/// A side of the canvas, named for the landscape canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The long side, which the long trims eat into.
    Long,
    /// The short side, which the short trims eat into.
    Short,
}

impl Side {
    const ALL: [Side; 2] = [Side::Long, Side::Short];

    /// The lowercase name, as the TOML keys spell it.
    fn name(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
        }
    }

    /// The length of this side in `profile`, in mm.
    fn canvas_mm(self, profile: &Profile) -> f64 {
        match self {
            Self::Long => profile.canvas_long_mm,
            Self::Short => profile.canvas_short_mm,
        }
    }

    /// The A and B trims on this side.
    fn trims(self) -> [Trim; 2] {
        match self {
            Self::Long => [Trim::LongA, Trim::LongB],
            Self::Short => [Trim::ShortA, Trim::ShortB],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::postcard;
    #[test]
    fn with_trims_sets_the_trims_on_the_named_edges() {
        let before = postcard();
        let trims = [
            (Edge::Left, 3.0),
            (Edge::Top, 4.0),
            (Edge::Right, 2.0),
            (Edge::Bottom, 5.0),
        ];
        for orientation in Orientation::ALL {
            let after = before.with_trims(orientation, &trims).unwrap();
            for (edge, mm) in trims {
                assert_eq!(
                    Trim::at(orientation, edge).mm(&after),
                    mm,
                    "{orientation:?} {edge:?}"
                );
            }
            assert_eq!(after.canvas_long_mm, before.canvas_long_mm);
            assert_eq!(after.max_stretch_pct, before.max_stretch_pct);
        }
    }

    #[test]
    fn with_trims_keeps_the_other_edges_and_leaves_self_alone() {
        let before = postcard();
        let after = before
            .with_trims(Orientation::Portrait, &[(Edge::Top, 3.5)])
            .unwrap();
        assert_eq!(before, postcard());
        // portrait top = long A
        assert_eq!(after.trim_long_a_mm, 3.5);
        assert_eq!(
            Profile {
                trim_long_a_mm: before.trim_long_a_mm,
                ..after
            },
            before
        );
    }

    #[test]
    fn with_trims_rejects_a_negative_trim_by_its_edge() {
        let err = postcard()
            .with_trims(Orientation::Landscape, &[(Edge::Bottom, -0.5)])
            .unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "the bottom trim is -0.5 mm: it must be 0 or more"
        );
    }

    #[test]
    fn with_trims_rejects_trims_that_leave_nothing_by_their_edges() {
        let err = postcard()
            .with_trims(
                Orientation::Portrait,
                &[(Edge::Left, 50.0), (Edge::Right, 50.0)],
            )
            .unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "the left and right trims (100 mm) leave nothing of the 100 mm side"
        );
    }
}
