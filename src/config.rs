//! The printer's measured geometry and the layout limits, stored as TOML at
//! `~/.config/selphy/printer.toml`. A missing file means "use the defaults",
//! which are the values measured on the first SELPHY CP1500 this ran on.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::atomic;
use crate::geometry::mm_to_px;

/// Trims are millimetres of canvas the printer loses on each edge in
/// Borderless mode. They are named for the LANDSCAPE canvas: long A is the
/// left end, long B the right end, short A the top edge, short B the bottom.
/// `geometry` maps them onto a portrait canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The canvas handed to the printer. 150x100mm maps cleanly onto postcard
    /// stock (100x148mm after the tabs are torn off). NOT 4x6 inch.
    pub canvas_long_mm: f64,
    pub canvas_short_mm: f64,
    pub trim_long_a_mm: f64,
    pub trim_long_b_mm: f64,
    pub trim_short_a_mm: f64,
    pub trim_short_b_mm: f64,
    /// Largest one-axis stretch, in percent, used to close the gap between
    /// the picture's aspect and the card's. 2:3 needs 1.9%.
    pub max_stretch_pct: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            canvas_long_mm: 150.0,
            canvas_short_mm: 100.0,
            trim_long_a_mm: 4.5,
            trim_long_b_mm: 5.5,
            trim_short_a_mm: 2.1,
            trim_short_b_mm: 2.7,
            max_stretch_pct: 2.5,
        }
    }
}

const HEADER: &str = "\
# selphy printer geometry. selphy rewrites this file, so comments added by
# hand are not kept.
# Trims are mm of canvas lost per edge, named for the landscape canvas:
# long A = left, long B = right, short A = top, short B = bottom.

";

impl Config {
    /// Reads the config at `path`. A missing file gives the defaults; keys
    /// missing from the file take their default value. Values that
    /// [`validate`](Self::validate) rejects are an error.
    pub fn load(path: &Path) -> Result<Self> {
        let cfg: Self = match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        cfg.validate()
            .with_context(|| format!("checking {}", path.display()))?;
        Ok(cfg)
    }

    /// Writes the config to `path`, creating its folder. Values that
    /// [`validate`](Self::validate) rejects are an error, and nothing is
    /// written.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let body = toml::to_string_pretty(self)?;
        atomic::write(path, format!("{HEADER}{body}"))
    }

    /// Checks that the values describe a printable canvas: the canvas sides
    /// are more than 0, the trims and the stretch are 0 or more, and on each
    /// side the two trims leave at least one pixel.
    pub fn validate(&self) -> Result<()> {
        let positive = [
            ("canvas_long_mm", self.canvas_long_mm),
            ("canvas_short_mm", self.canvas_short_mm),
        ];
        for (key, value) in positive {
            ensure!(
                value.is_finite() && value > 0.0,
                "{key} = {value}: must be more than 0"
            );
        }
        let non_negative = [
            ("trim_long_a_mm", self.trim_long_a_mm),
            ("trim_long_b_mm", self.trim_long_b_mm),
            ("trim_short_a_mm", self.trim_short_a_mm),
            ("trim_short_b_mm", self.trim_short_b_mm),
            ("max_stretch_pct", self.max_stretch_pct),
        ];
        for (key, value) in non_negative {
            ensure!(
                value.is_finite() && value >= 0.0,
                "{key} = {value}: must be 0 or more"
            );
        }
        let sides = [
            (
                "long",
                self.canvas_long_mm,
                self.trim_long_a_mm,
                self.trim_long_b_mm,
            ),
            (
                "short",
                self.canvas_short_mm,
                self.trim_short_a_mm,
                self.trim_short_b_mm,
            ),
        ];
        for (side, canvas, a, b) in sides {
            ensure!(
                mm_to_px(canvas) - mm_to_px(a) - mm_to_px(b) >= 1,
                "trim_{side}_a_mm + trim_{side}_b_mm = {} mm leaves nothing of the \
                 {canvas} mm canvas_{side}_mm",
                a + b
            );
        }
        Ok(())
    }
}

/// `$SELPHY_CONFIG` if set, else `$XDG_CONFIG_HOME/selphy/printer.toml`,
/// else `~/.config/selphy/printer.toml`.
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SELPHY_CONFIG") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("selphy").join("printer.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::fresh_dir;

    fn temp_path(name: &str) -> PathBuf {
        fresh_dir(name).join("printer.toml")
    }

    #[test]
    fn missing_file_gives_defaults() {
        let cfg = Config::load(&temp_path("missing")).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_path("roundtrip");
        let cfg = Config {
            trim_short_a_mm: 1.8,
            ..Config::default()
        };
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);
    }

    #[test]
    fn missing_keys_take_defaults() {
        let path = temp_path("partial");
        fs::write(&path, "trim_long_a_mm = 4.0\n").unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.trim_long_a_mm, 4.0);
        assert_eq!(cfg.trim_long_b_mm, Config::default().trim_long_b_mm);
    }

    #[test]
    fn values_that_break_the_layout_are_rejected() {
        let cases = [
            (
                "canvas_short_mm = -5.0",
                "canvas_short_mm = -5: must be more than 0",
            ),
            (
                "canvas_long_mm = nan",
                "canvas_long_mm = NaN: must be more than 0",
            ),
            ("trim_top_mm = 1.0", "trim_top_mm"),
            (
                "trim_short_a_mm = -0.1",
                "trim_short_a_mm = -0.1: must be 0 or more",
            ),
            (
                "max_stretch_pct = -200.0",
                "max_stretch_pct = -200: must be 0 or more",
            ),
            (
                "trim_long_a_mm = 200.0",
                "leaves nothing of the 150 mm canvas_long_mm",
            ),
        ];
        for (i, (text, reason)) in cases.into_iter().enumerate() {
            let path = temp_path(&format!("invalid-{i}"));
            fs::write(&path, format!("{text}\n")).unwrap();
            let err = Config::load(&path).unwrap_err();
            assert!(format!("{err:#}").contains(reason), "{text}: {err:#}");
        }
    }

    #[test]
    fn invalid_values_are_not_saved() {
        let path = temp_path("save-invalid");
        let cfg = Config {
            trim_short_a_mm: 99.0,
            ..Config::default()
        };
        assert!(cfg.save(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let path = temp_path("typo");
        fs::write(&path, "trim_lng_a_mm = 4.0\n").unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("trim_lng_a_mm"), "{err:#}");
    }
}
