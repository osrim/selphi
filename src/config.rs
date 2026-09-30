//! The printer's measured geometry and the layout limits, stored as TOML at
//! `~/.config/selphy/printer.toml`. A missing file means "use the defaults",
//! which are the values measured on the first SELPHY CP1500 this ran on.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::atomic;

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
    /// missing from the file take their default value.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let body = toml::to_string_pretty(self)?;
        atomic::write(path, format!("{HEADER}{body}"))
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
    fn unknown_keys_are_rejected() {
        let path = temp_path("typo");
        fs::write(&path, "trim_lng_a_mm = 4.0\n").unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("trim_lng_a_mm"), "{err:#}");
    }
}
