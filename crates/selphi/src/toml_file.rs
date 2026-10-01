//! Settings files stored as TOML: the printer config and the window's own
//! settings. A missing file means "use the defaults", and a save never leaves
//! a partial file.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::atomic;

/// Reads `path` as TOML. A missing file gives `T::default()`.
pub fn load_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Writes `value` to `path` as TOML, after `header`. Creates the folder, and
/// replaces the file atomically.
pub fn save<T: Serialize>(path: &Path, header: &str, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let body = toml::to_string_pretty(value)?;
    atomic::write(path, format!("{header}{body}"))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::test_util::fresh_dir;

    #[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Theme {
        #[default]
        System,
        Dark,
    }

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Settings {
        theme: Theme,
    }

    #[test]
    fn a_missing_file_gives_the_default() {
        let path = fresh_dir("toml-missing").join("gui.toml");
        let settings: Settings = load_or_default(&path).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn save_then_load_round_trips_and_leaves_no_temporary() {
        let dir = fresh_dir("toml-roundtrip");
        let path = dir.join("sub").join("gui.toml");
        let settings = Settings { theme: Theme::Dark };
        save(&path, "# header\n", &settings).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# header\ntheme = \"dark\"\n"
        );
        assert_eq!(load_or_default::<Settings>(&path).unwrap(), settings);
        let names: Vec<_> = fs::read_dir(dir.join("sub"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["gui.toml"]);
    }

    #[test]
    fn a_parse_error_names_the_file() {
        let path = fresh_dir("toml-unknown").join("gui.toml");
        fs::write(&path, "theme = \"blue\"\n").unwrap();
        let err = load_or_default::<Settings>(&path).unwrap_err();
        assert!(
            format!("{err:#}").starts_with(&format!("parsing {}", path.display())),
            "{err:#}"
        );
    }
}
