//! The printer config file, and the env vars that override its values for
//! one run. Values resolve in this order: env, then file, then defaults. An
//! override is never written to the file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::Config;
use super::fields::{FIELDS, Field};
use crate::toml_file;

const HEADER: &str = "\
# selphy printer geometry. selphy rewrites this file, so comments added by
# hand are not kept.
# Trims are mm of canvas lost per edge, named for the landscape canvas:
# long A = left, long B = right, short A = top, short B = bottom.

";

/// The printer config file, and the env overrides for this run.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: PathBuf,
    /// The raw values of the env vars that are set. `load` parses them.
    env_values: Vec<(&'static Field, OsString)>,
}

/// The config read from the file, and the values this run uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The file's values over the defaults. Start from this to edit the
    /// file.
    pub saved: Config,
    /// `saved` with the env overrides applied. Use this to prepare photos.
    pub effective: Config,
    /// The overrides that are set, in the order of the field table.
    pub overrides: Vec<Override>,
}

/// One env var that overrides a field for this run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Override {
    /// The field it overrides. Its env var is `field.env`.
    pub field: &'static Field,
    /// The value from the env var.
    pub value: f64,
}

impl ConfigFile {
    /// The config file this process uses: `explicit` if given, else
    /// `$XDG_CONFIG_HOME/selphy/printer.toml`, else
    /// `~/.config/selphy/printer.toml`. Reads the env overrides once, now.
    pub fn locate(explicit: Option<PathBuf>) -> ConfigFile {
        let path = explicit.unwrap_or_else(|| {
            let base = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                .unwrap_or_else(|| PathBuf::from(".config"));
            base.join("selphy").join("printer.toml")
        });
        let env_values = FIELDS
            .iter()
            .filter_map(|&field| Some((field, std::env::var_os(field.env)?)))
            .collect();
        ConfigFile { path, env_values }
    }

    /// The config file at `path`, with no overrides.
    pub fn at(path: impl Into<PathBuf>) -> ConfigFile {
        ConfigFile {
            path: path.into(),
            env_values: Vec::new(),
        }
    }

    /// This file with the overrides in `pairs`: env var names and their
    /// values, as if read from the environment.
    ///
    /// # Panics
    ///
    /// If a name is not the env var of a field.
    pub fn with_overrides<'a>(
        mut self,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        for (name, value) in pairs {
            let field = FIELDS
                .into_iter()
                .find(|field| field.env == name)
                .unwrap_or_else(|| panic!("{name} is not a config env var"));
            self.env_values.push((field, value.into()));
        }
        self
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file exists.
    pub fn exists(&self) -> Result<bool> {
        self.path
            .try_exists()
            .with_context(|| format!("checking {}", self.path.display()))
    }

    /// The file `name` in the config file's folder.
    pub fn sibling(&self, name: &str) -> PathBuf {
        self.path.with_file_name(name)
    }

    /// Reads the file and applies the overrides. A missing file gives the
    /// defaults; keys missing from the file take their default value.
    ///
    /// The file's values and the values with the overrides are validated
    /// apart, so that an error names its cause: the file, or the env vars.
    pub fn load(&self) -> Result<Loaded> {
        let saved: Config = toml_file::load_or_default(&self.path)?;
        saved
            .validate()
            .with_context(|| format!("checking {}", self.path.display()))?;
        let overrides = self.parse_overrides()?;
        let mut effective = saved.clone();
        for o in &overrides {
            *o.field.get_mut(&mut effective) = o.value;
        }
        if !overrides.is_empty() {
            effective.validate().with_context(|| {
                let names: Vec<&str> = overrides.iter().map(|o| o.field.env).collect();
                format!(
                    "checking {} with {} set",
                    self.path.display(),
                    names.join(", ")
                )
            })?;
        }
        Ok(Loaded {
            saved,
            effective,
            overrides,
        })
    }

    /// Writes `cfg` to the file, creating its folder. Values that
    /// [`Config::validate`] rejects are an error, and nothing is written.
    pub fn save(&self, cfg: &Config) -> Result<()> {
        cfg.validate()?;
        toml_file::save(&self.path, HEADER, cfg)
    }

    /// The overrides that are set, in the order of the field table. An empty
    /// value counts as not set.
    fn parse_overrides(&self) -> Result<Vec<Override>> {
        let mut parsed = Vec::new();
        for field in FIELDS {
            // The last value wins, as in a shell.
            let Some((_, raw)) = self.env_values.iter().rev().find(|(f, _)| *f == field) else {
                continue;
            };
            let text = raw.to_string_lossy();
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            match text.parse::<f64>() {
                Ok(value) if value.is_finite() => parsed.push(Override { field, value }),
                _ => bail!("{} = {text}: not a number", field.env),
            }
        }
        Ok(parsed)
    }
}

impl Loaded {
    /// The override on `field`, if one is set.
    pub fn override_of(&self, field: &Field) -> Option<&Override> {
        self.overrides.iter().find(|o| o.field == field)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::config::fields::{MAX_STRETCH, TRIM_LONG_A};
    use crate::test_util::fresh_dir;

    fn temp_file(name: &str) -> ConfigFile {
        ConfigFile::at(fresh_dir(name).join("printer.toml"))
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let loaded = temp_file("config-missing").load().unwrap();
        assert_eq!(loaded.saved, Config::default());
        assert_eq!(loaded.effective, Config::default());
        assert!(loaded.overrides.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let file = temp_file("config-roundtrip");
        let cfg = Config {
            trim_short_a_mm: 1.8,
            ..Config::default()
        };
        file.save(&cfg).unwrap();
        assert!(file.exists().unwrap());
        assert_eq!(file.load().unwrap().saved, cfg);
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.starts_with("# selphy printer geometry."), "{text}");
    }

    #[test]
    fn missing_keys_take_defaults() {
        let file = temp_file("config-partial");
        fs::write(file.path(), "trim_long_a_mm = 4.0\n").unwrap();
        let cfg = file.load().unwrap().saved;
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
            let file = temp_file(&format!("config-invalid-{i}"));
            fs::write(file.path(), format!("{text}\n")).unwrap();
            let err = file.load().unwrap_err();
            assert!(format!("{err:#}").contains(reason), "{text}: {err:#}");
        }
    }

    #[test]
    fn invalid_values_are_not_saved() {
        let file = temp_file("config-save-invalid");
        let cfg = Config {
            trim_short_a_mm: 99.0,
            ..Config::default()
        };
        assert!(file.save(&cfg).is_err());
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let file = temp_file("config-typo");
        fs::write(file.path(), "trim_lng_a_mm = 4.0\n").unwrap();
        let err = file.load().unwrap_err();
        assert!(format!("{err:#}").contains("trim_lng_a_mm"), "{err:#}");
    }

    #[test]
    fn an_override_changes_effective_and_not_saved() {
        let file = temp_file("config-override").with_overrides([
            ("SELPHY_TRIM_LONG_A_MM", "3.25"),
            ("SELPHY_MAX_STRETCH_PCT", "0"),
        ]);
        let loaded = file.load().unwrap();
        assert_eq!(loaded.saved, Config::default());
        assert_eq!(
            loaded.effective,
            Config {
                trim_long_a_mm: 3.25,
                max_stretch_pct: 0.0,
                ..Config::default()
            }
        );
        assert_eq!(
            loaded.overrides,
            [
                Override {
                    field: &TRIM_LONG_A,
                    value: 3.25
                },
                Override {
                    field: &MAX_STRETCH,
                    value: 0.0
                },
            ]
        );
        assert_eq!(loaded.override_of(&MAX_STRETCH).unwrap().value, 0.0);
    }

    #[test]
    fn an_empty_override_is_ignored() {
        let file =
            temp_file("config-override-empty").with_overrides([("SELPHY_TRIM_LONG_A_MM", "")]);
        let loaded = file.load().unwrap();
        assert_eq!(loaded.effective, Config::default());
        assert!(loaded.overrides.is_empty());
    }

    #[test]
    fn an_override_that_is_not_a_number_names_the_variable() {
        for value in ["abc", "inf", "NaN"] {
            let file =
                temp_file("config-override-nan").with_overrides([("SELPHY_TRIM_LONG_A_MM", value)]);
            let err = file.load().unwrap_err();
            assert_eq!(
                format!("{err:#}"),
                format!("SELPHY_TRIM_LONG_A_MM = {value}: not a number")
            );
        }
    }

    #[test]
    fn an_override_that_breaks_the_layout_names_the_variable() {
        let file =
            temp_file("config-override-invalid").with_overrides([("SELPHY_TRIM_LONG_A_MM", "200")]);
        fs::write(file.path(), "trim_long_a_mm = 4.0\n").unwrap();
        let err = format!("{:#}", file.load().unwrap_err());
        assert!(err.contains("with SELPHY_TRIM_LONG_A_MM set"), "{err}");
        assert!(err.contains("leaves nothing of the 150 mm"), "{err}");
    }

    #[test]
    fn save_of_saved_does_not_write_the_override() {
        let file =
            temp_file("config-override-save").with_overrides([("SELPHY_TRIM_LONG_A_MM", "3.25")]);
        let loaded = file.load().unwrap();
        file.save(&loaded.saved).unwrap();
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 4.5"), "{text}");
        assert_eq!(
            ConfigFile::at(file.path()).load().unwrap().saved,
            Config::default()
        );
    }

    #[test]
    fn sibling_is_in_the_config_folder() {
        let file = ConfigFile::at("/etc/selphy/printer.toml");
        assert_eq!(file.sibling("gui.toml"), Path::new("/etc/selphy/gui.toml"));
    }

    /// The one test that reads and sets real env vars, so that no other test
    /// races with it.
    #[test]
    fn locate_follows_the_path_rules_and_reads_the_env() {
        let saved: Vec<_> = ["XDG_CONFIG_HOME", "HOME", "SELPHY_TRIM_LONG_A_MM"]
            .map(|name| (name, std::env::var_os(name)))
            .into();
        // SAFETY: no other test in this process reads or writes these vars.
        unsafe {
            std::env::set_var("HOME", "/home/me");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::set_var("SELPHY_TRIM_LONG_A_MM", "3.5");
        }
        let home = ConfigFile::locate(None);
        // SAFETY: as above.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/xdg") };
        let xdg = ConfigFile::locate(None);
        let mine = fresh_dir("config-locate").join("mine.toml");
        let explicit = ConfigFile::locate(Some(mine.clone()));
        // SAFETY: as above.
        unsafe {
            for (name, value) in saved {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }

        assert_eq!(
            home.path(),
            Path::new("/home/me/.config/selphy/printer.toml")
        );
        assert_eq!(xdg.path(), Path::new("/xdg/selphy/printer.toml"));
        assert_eq!(explicit.path(), mine);
        let overrides = &explicit.load().unwrap().overrides;
        assert_eq!(
            overrides,
            &[Override {
                field: &TRIM_LONG_A,
                value: 3.5
            }]
        );
    }
}
