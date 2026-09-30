//! The SELPHY paper sizes, and the profile each one starts from. A new paper
//! is added here, and gets its table in `config::Config`.

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Profile;

/// A SELPHY paper size. It names the physical card, not the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Paper {
    /// Postcard, 148 x 100 mm.
    Postcard,
    /// L, 119 x 89 mm.
    L,
    /// Card (credit-card size), 86 x 54 mm.
    Card,
}

impl Paper {
    /// Every paper, in the order of the config file's tables.
    pub const ALL: [Paper; 3] = [Paper::Postcard, Paper::L, Paper::Card];

    /// The lowercase name, as the command line, the config file and the
    /// placement record spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Postcard => "postcard",
            Self::L => "l",
            Self::Card => "card",
        }
    }

    /// The card's (long, short) sides in mm, from the Canon CP1500 manual.
    pub fn size_mm(self) -> (f64, f64) {
        match self {
            Self::Postcard => (148.0, 100.0),
            Self::L => (119.0, 89.0),
            Self::Card => (86.0, 54.0),
        }
    }

    /// The built-in profile. Postcard has the values measured on the first
    /// SELPHY CP1500 this ran on. The other papers have none: they must be
    /// calibrated.
    pub fn default_profile(self) -> Option<Profile> {
        match self {
            Self::Postcard => Some(Profile {
                canvas_long_mm: 150.0,
                canvas_short_mm: 100.0,
                trim_long_a_mm: 4.5,
                trim_long_b_mm: 5.5,
                trim_short_a_mm: 2.1,
                trim_short_b_mm: 2.7,
                max_stretch_pct: 2.5,
            }),
            Self::L | Self::Card => None,
        }
    }

    /// The profile that calibration starts from: the built-in profile, else
    /// a canvas the size of the card, with no trims and a 2.5 % stretch.
    pub fn starting_profile(self) -> Profile {
        self.default_profile().unwrap_or_else(|| {
            let (long, short) = self.size_mm();
            Profile {
                canvas_long_mm: long,
                canvas_short_mm: short,
                trim_long_a_mm: 0.0,
                trim_long_b_mm: 0.0,
                trim_short_a_mm: 0.0,
                trim_short_b_mm: 0.0,
                max_stretch_pct: 2.5,
            }
        })
    }
}

impl fmt::Display for Paper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Paper {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match Paper::ALL.into_iter().find(|paper| paper.name() == s) {
            Some(paper) => Ok(paper),
            None => {
                let names: Vec<&str> = Paper::ALL.map(Paper::name).into();
                bail!("unknown paper {s:?}; the papers are {}", names.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        paper: Paper,
    }

    #[test]
    fn the_names_round_trip_through_serde_and_from_str() {
        for paper in Paper::ALL {
            let text = toml::to_string(&Holder { paper }).unwrap();
            assert_eq!(text, format!("paper = \"{}\"\n", paper.name()));
            assert_eq!(toml::from_str::<Holder>(&text).unwrap().paper, paper);
            assert_eq!(paper.name().parse::<Paper>().unwrap(), paper);
            assert_eq!(paper.to_string(), paper.name());
        }
        assert_eq!(Paper::ALL.map(Paper::name), ["postcard", "l", "card"]);
    }

    #[test]
    fn an_unknown_name_lists_the_papers() {
        let err = "a4".parse::<Paper>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "unknown paper \"a4\"; the papers are postcard, l, card"
        );
    }

    #[test]
    fn postcard_has_the_measured_defaults() {
        let postcard = Paper::Postcard.default_profile().unwrap();
        assert_eq!(
            (postcard.canvas_long_mm, postcard.canvas_short_mm),
            (150.0, 100.0)
        );
        assert_eq!(
            [
                postcard.trim_long_a_mm,
                postcard.trim_long_b_mm,
                postcard.trim_short_a_mm,
                postcard.trim_short_b_mm
            ],
            [4.5, 5.5, 2.1, 2.7]
        );
        assert_eq!(postcard.max_stretch_pct, 2.5);
        assert_eq!(Paper::Postcard.starting_profile(), postcard);
    }

    #[test]
    fn l_and_card_are_not_calibrated_and_start_at_the_paper_size() {
        for paper in [Paper::L, Paper::Card] {
            assert_eq!(paper.default_profile(), None, "{paper}");
            let start = paper.starting_profile();
            assert_eq!(
                (start.canvas_long_mm, start.canvas_short_mm),
                paper.size_mm(),
                "{paper}"
            );
            let trims = [
                start.trim_long_a_mm,
                start.trim_long_b_mm,
                start.trim_short_a_mm,
                start.trim_short_b_mm,
            ];
            assert_eq!(trims, [0.0; 4], "{paper}");
            assert_eq!(start.max_stretch_pct, 2.5);
            start.validate().unwrap();
        }
    }
}
