//! The window's own settings, stored as TOML in `gui.toml` next to the
//! printer config. The command line never reads this file.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Window};
use serde::{Deserialize, Serialize};

use selphy::config;

/// Which colors the window uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Follow the system appearance.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    /// In the order the Config dialog lists them.
    pub const ALL: [ThemeChoice; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// Sets the global theme. Call again for `System` when the system
    /// appearance changes.
    pub fn apply(self, window: &mut Window, cx: &mut App) {
        match self {
            Self::System => Theme::sync_system_appearance(Some(window), cx),
            Self::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            Self::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub theme: ThemeChoice,
}

impl Appearance {
    /// Reads `path`. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Writes `path`, creating its folder.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        fs::write(path, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// `gui.toml` in the folder of the printer config.
pub fn default_path() -> PathBuf {
    let printer = config::default_path();
    printer
        .parent()
        .map_or_else(|| PathBuf::from("gui.toml"), |dir| dir.join("gui.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("selphy-gui-{name}-{}", std::process::id()));
        dir.join("gui.toml")
    }

    #[test]
    fn missing_file_follows_the_system() {
        let appearance = Appearance::load(&temp_path("missing")).unwrap();
        assert_eq!(appearance.theme, ThemeChoice::System);
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_path("roundtrip");
        let appearance = Appearance {
            theme: ThemeChoice::Dark,
        };
        appearance.save(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "theme = \"dark\"\n");
        assert_eq!(Appearance::load(&path).unwrap(), appearance);
    }

    #[test]
    fn unknown_theme_is_an_error() {
        let path = temp_path("unknown");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "theme = \"blue\"\n").unwrap();
        assert!(Appearance::load(&path).is_err());
    }
}
