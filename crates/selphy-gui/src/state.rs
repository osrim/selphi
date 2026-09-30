//! The window's own settings, stored as TOML in `gui.toml` next to the
//! printer config through `selphy::toml_file`, and [`Prefs`], the one copy
//! of them that both windows share. The command line never reads `gui.toml`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeMode};
use selphy::config::ConfigFile;
use selphy::geometry::Fit;
use selphy::paper::Paper;
use selphy::toml_file;
use serde::{Deserialize, Serialize};

/// The settings file's name, in the printer config's folder.
pub const FILE_NAME: &str = "gui.toml";

/// Which colors the windows use.
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
    /// In the order the Appearance pane lists them.
    pub const ALL: [ThemeChoice; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// Sets the global theme, which every window follows. Call again for
    /// `System` when the system appearance changes.
    pub fn apply(self, cx: &mut App) {
        match self {
            Self::System => Theme::sync_system_appearance(None, cx),
            Self::Light => Theme::change(ThemeMode::Light, None, cx),
            Self::Dark => Theme::change(ThemeMode::Dark, None, cx),
        }
    }
}

/// `gui.toml`: the theme, and the paper, fit and output folder of the last
/// session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuiSettings {
    pub theme: ThemeChoice,
    pub paper: Paper,
    pub fit: Fit,
    pub out_dir: PathBuf,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            paper: Paper::Postcard,
            fit: Fit::Contain,
            out_dir: default_out_dir(),
        }
    }
}

impl GuiSettings {
    /// Reads `path`. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Self> {
        toml_file::load_or_default(path)
    }

    /// Writes `path`, atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        toml_file::save(path, "", self)
    }
}

/// `~/Pictures/SELPHY`, so that the prints do not mix with other pictures.
/// Preparing creates it.
pub fn default_out_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Pictures").join("SELPHY")
}

/// The window settings and the printer config file, shared by the main
/// window and the Settings window. The revision goes up when the paper, the
/// fit or the saved profile changes, so that previews made for an older
/// revision can be dropped.
#[derive(Debug)]
pub struct Prefs {
    config: ConfigFile,
    settings: GuiSettings,
    revision: u64,
}

impl Prefs {
    pub fn new(config: ConfigFile, settings: GuiSettings) -> Self {
        Self {
            config,
            settings,
            revision: 0,
        }
    }

    pub fn config(&self) -> &ConfigFile {
        &self.config
    }

    pub fn settings(&self) -> &GuiSettings {
        &self.settings
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Where `gui.toml` is.
    pub fn path(&self) -> PathBuf {
        self.config.sibling(FILE_NAME)
    }

    /// Applies `change` and saves `gui.toml`. The change is kept when the
    /// save fails, so that the window still does what the user chose.
    pub fn change(&mut self, change: impl FnOnce(&mut GuiSettings)) -> Result<()> {
        let before = (self.settings.paper, self.settings.fit);
        change(&mut self.settings);
        if (self.settings.paper, self.settings.fit) != before {
            self.revision += 1;
        }
        self.settings.save(&self.path())
    }

    /// Marks the printer config as saved, so that previews are made again.
    pub fn printer_saved(&mut self) {
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use selphy::test_util::fresh_dir;

    use super::*;

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let path = fresh_dir("gui-settings-missing").join(FILE_NAME);
        let settings = GuiSettings::load(&path).unwrap();
        assert_eq!(settings, GuiSettings::default());
        assert_eq!(settings.paper, Paper::Postcard);
        assert_eq!(settings.fit, Fit::Contain);
        assert_eq!(settings.theme, ThemeChoice::System);
        assert!(settings.out_dir.ends_with("Pictures/SELPHY"));
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let path = fresh_dir("gui-settings-roundtrip").join(FILE_NAME);
        let settings = GuiSettings {
            theme: ThemeChoice::Dark,
            paper: Paper::Card,
            fit: Fit::Cover,
            out_dir: PathBuf::from("/tmp/prints"),
        };
        settings.save(&path).unwrap();
        assert_eq!(GuiSettings::load(&path).unwrap(), settings);
    }

    #[test]
    fn a_file_with_only_a_theme_keeps_the_other_defaults() {
        let path = fresh_dir("gui-settings-theme-only").join(FILE_NAME);
        std::fs::write(&path, "theme = \"light\"\n").unwrap();
        let settings = GuiSettings::load(&path).unwrap();
        assert_eq!(settings.theme, ThemeChoice::Light);
        assert_eq!(settings.paper, Paper::Postcard);
    }

    #[test]
    fn the_revision_follows_paper_and_fit_but_not_the_folder() {
        let dir = fresh_dir("gui-prefs-revision");
        let mut prefs = Prefs::new(
            ConfigFile::at(dir.join("printer.toml")),
            GuiSettings::default(),
        );
        prefs.change(|s| s.out_dir = dir.join("out")).unwrap();
        assert_eq!(prefs.revision(), 0);
        prefs.change(|s| s.paper = Paper::L).unwrap();
        assert_eq!(prefs.revision(), 1);
        prefs.change(|s| s.fit = Fit::Cover).unwrap();
        prefs.printer_saved();
        assert_eq!(prefs.revision(), 3);
        assert_eq!(GuiSettings::load(&prefs.path()).unwrap(), *prefs.settings());
    }
}
