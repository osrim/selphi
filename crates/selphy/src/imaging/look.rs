//! The output settings that change the pixels, not the placement: how much
//! the picture is sharpened, and the colour around a contain picture. The
//! colour space is not a setting: the output is always sRGB, because the
//! SELPHY does no colour management.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The output settings of one job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    /// How much the resized picture is sharpened.
    pub sharpening: Sharpening,
    /// The colour of the canvas around the picture.
    pub background: Background,
}

/// The unsharp mask after resizing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Sharpening {
    /// No sharpening.
    Off,
    /// A light mask that offsets the softening of the resize.
    #[default]
    Standard,
    /// More, for soft photos.
    Strong,
}

impl Sharpening {
    /// Every sharpening.
    pub const ALL: [Sharpening; 3] = [Sharpening::Off, Sharpening::Standard, Sharpening::Strong];

    /// The lowercase name, as the command line and the config file spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Standard => "standard",
            Self::Strong => "strong",
        }
    }

    /// The name the user sees in the window and in `selphy config`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Standard => "Standard",
            Self::Strong => "Strong",
        }
    }

    /// How much of the detail is added back: 0 for off.
    pub(crate) fn amount(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Standard => 0.75,
            Self::Strong => 1.25,
        }
    }
}

/// The colour of the canvas where the picture is not. It shows on the card
/// only around a contain picture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Background {
    /// White, the colour of the paper.
    #[default]
    White,
    /// Black.
    Black,
}

impl Background {
    /// Every background.
    pub const ALL: [Background; 2] = [Background::White, Background::Black];

    /// The lowercase name, as the command line and the config file spell it.
    pub fn name(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Black => "black",
        }
    }

    /// The name the user sees in the window and in `selphy config`.
    pub fn label(self) -> &'static str {
        match self {
            Self::White => "White",
            Self::Black => "Black",
        }
    }

    pub(crate) fn rgb(self) -> image::Rgb<u8> {
        match self {
            Self::White => image::Rgb([255; 3]),
            Self::Black => image::Rgb([0; 3]),
        }
    }
}

impl fmt::Display for Sharpening {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for Background {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        sharpening: Sharpening,
        background: Background,
    }

    #[test]
    fn names_round_trip_through_serde() {
        for sharpening in Sharpening::ALL {
            for background in Background::ALL {
                let holder = Holder {
                    sharpening,
                    background,
                };
                let text = toml::to_string(&holder).unwrap();
                assert_eq!(
                    text,
                    format!("sharpening = \"{sharpening}\"\nbackground = \"{background}\"\n")
                );
                assert_eq!(toml::from_str::<Holder>(&text).unwrap(), holder);
            }
        }
    }

    #[test]
    fn the_default_look_is_the_one_before_there_were_settings() {
        let look = Look::default();
        assert_eq!(look.sharpening.amount(), 0.75);
        assert_eq!(look.background.rgb(), image::Rgb([255; 3]));
    }
}
