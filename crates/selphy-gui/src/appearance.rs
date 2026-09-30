//! The window's own settings, stored as TOML in `gui.toml` next to the
//! printer config, through `selphy::toml_file`. The command line never reads
//! this file.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Window};
use serde::{Deserialize, Serialize};

/// The settings file's name, in the printer config's folder.
pub const FILE_NAME: &str = "gui.toml";

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
