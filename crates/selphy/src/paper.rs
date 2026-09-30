//! The SELPHY paper size selphy prepares for, and its built-in profile.

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Profile;

/// A SELPHY paper size. It names the physical card, not the image. selphy
/// prepares for postcard paper only; the placement record names it, so that
/// another size can be told apart if it is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Paper {
    /// Postcard, 148 x 100 mm.
    Postcard,
}

impl Paper {
    /// Every paper.
    pub const ALL: [Paper; 1] = [Paper::Postcard];

    /// The lowercase name, as the config file and the placement record spell
    /// it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Postcard => "postcard",
        }
    }

    /// The built-in profile: the values measured on the first SELPHY CP1500
    /// this ran on.
    pub fn default_profile(self) -> Profile {
        match self {
            Self::Postcard => Profile {
                canvas_long_mm: 150.0,
                canvas_short_mm: 100.0,
                trim_long_a_mm: 4.5,
                trim_long_b_mm: 5.5,
                trim_short_a_mm: 2.1,
                trim_short_b_mm: 2.7,
                max_stretch_pct: 2.5,
            },
        }
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
    fn the_name_round_trips_through_serde_and_from_str() {
        let text = toml::to_string(&Holder {
            paper: Paper::Postcard,
        })
        .unwrap();
        assert_eq!(text, "paper = \"postcard\"\n");
        assert_eq!(
            toml::from_str::<Holder>(&text).unwrap().paper,
            Paper::Postcard
        );
        assert_eq!("postcard".parse::<Paper>().unwrap(), Paper::Postcard);
        assert_eq!(Paper::Postcard.to_string(), "postcard");
    }

    #[test]
    fn an_unknown_name_lists_the_papers() {
        let err = "card".parse::<Paper>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "unknown paper \"card\"; the papers are postcard"
        );
    }

    #[test]
    fn postcard_has_the_measured_defaults() {
        let postcard = Paper::Postcard.default_profile();
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
    }
}
