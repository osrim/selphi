//! The printer config file, and the env vars that override its values for
//! one run. An override is never written to the file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};

use super::fields::{FIELDS, Field};
use super::{Config, Profile};
use crate::geometry::Fit;
use crate::paper::Paper;
use crate::toml_file;

/// The env var that names the fit.
pub const FIT_ENV: &str = "SELPHY_FIT";

const HEADER: &str = "\
# selphy printer geometry: the default fit, the output settings
# (sharpening = off | standard | strong, background = white | black),
# and the [postcard] table.
# selphy rewrites this file, so comments added by hand are not kept.
# Trims are mm of canvas lost per edge, named for the landscape canvas:
# long A = left, long B = right, short A = top, short B = bottom.

";

/// The printer config file, and the env overrides for this run.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: PathBuf,
    /// Raw, so that a bad value is an error from `load`, not from `locate`.
    env_values: Vec<(&'static Field, OsString)>,
    env_fit: Option<OsString>,
}

/// The config read from the file, the fit this run uses, and the overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The file's values. Start from this to edit the file.
    pub saved: Config,
    /// The fit this run uses.
    pub fit: Fit,
    /// The overrides that are set, in the order of the field table.
    pub overrides: Vec<Override>,
    path: PathBuf,
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
    /// `~/.config/selphy/printer.toml`. Reads the env overrides and
    /// `SELPHY_FIT` once, now.
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
        ConfigFile {
            path,
            env_values,
            env_fit: std::env::var_os(FIT_ENV),
        }
    }

    /// The config file at `path`, with no overrides.
    pub fn at(path: impl Into<PathBuf>) -> ConfigFile {
        ConfigFile {
            path: path.into(),
            env_values: Vec::new(),
            env_fit: None,
        }
    }

    /// This file with the env vars in `pairs`: names and values, as if read
    /// from the environment. A name is `SELPHY_FIT` or the env var of a
    /// field.
    ///
    /// # Panics
    ///
    /// If a name is neither.
    pub fn with_overrides<'a>(
        mut self,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        for (name, value) in pairs {
            if name == FIT_ENV {
                self.env_fit = Some(value.into());
                continue;
            }
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

    /// Reads and checks the file, parses the overrides, and resolves the fit:
    /// `fit` if given, else `SELPHY_FIT`, else the file's `fit`, else
    /// contain. A missing file gives the defaults.
    ///
    /// An old file with the profile keys at the top level is an error that
    /// says to move them into a `[postcard]` table.
    pub fn load(&self, fit: Option<Fit>) -> Result<Loaded> {
        let path = self.path.display();
        let table: toml::Table = toml_file::load_or_default(&self.path)?;
        if table.keys().any(|key| FIELDS.iter().any(|f| f.key == key)) {
            bail!(
                "{path}: the profile keys must be under [postcard]. Move them into a [postcard] \
                 table."
            );
        }
        let saved: Config = toml::Value::Table(table)
            .try_into()
            .with_context(|| format!("parsing {path}"))?;
        saved
            .validate()
            .with_context(|| format!("checking {path}"))?;
        let overrides = self.parse_overrides()?;
        let fit = match fit {
            Some(fit) => fit,
            None => parse_env(FIT_ENV, self.env_fit.as_ref())?
                .or(saved.fit)
                .unwrap_or(Fit::Contain),
        };
        Ok(Loaded {
            saved,
            fit,
            overrides,
            path: self.path.clone(),
        })
    }

    /// Writes `config` to the file, creating its folder. Values that
    /// [`Config::validate`] rejects are an error, and nothing is written.
    pub fn save(&self, config: &Config) -> Result<()> {
        config.validate()?;
        toml_file::save(&self.path, HEADER, config)
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

/// The value of the env var `name`, whose raw value is `raw`. An empty value
/// counts as not set.
fn parse_env<T>(name: &str, raw: Option<&OsString>) -> Result<Option<T>>
where
    T: FromStr<Err = anyhow::Error>,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let text = raw.to_string_lossy();
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .with_context(|| format!("{name} = {text}"))
}

impl Loaded {
    /// The file's profile, without the overrides. Start from this to edit
    /// the file.
    pub fn saved_profile(&self) -> Profile {
        self.saved.profile()
    }

    /// The profile this run uses: [`Loaded::saved_profile`] with the
    /// overrides applied. Use this to prepare photos.
    pub fn profile(&self) -> Result<Profile> {
        self.apply_overrides(self.saved_profile())
    }

    /// `profile` with the overrides applied. The result is validated, and an
    /// error names the env vars, so that it is not taken for an error in the
    /// file.
    pub fn apply_overrides(&self, mut profile: Profile) -> Result<Profile> {
        if self.overrides.is_empty() {
            return Ok(profile);
        }
        for o in &self.overrides {
            *o.field.get_mut(&mut profile) = o.value;
        }
        profile.validate().map_err(|invalid| {
            let names: Vec<&str> = self.overrides.iter().map(|o| o.field.env).collect();
            anyhow!(
                "checking {} with {} set: [{}] {invalid}",
                self.path.display(),
                names.join(", "),
                Paper::Postcard
            )
        })?;
        Ok(profile)
    }

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
    use crate::imaging::{Background, Look, Sharpening};
    use crate::test_util::{fresh_dir, postcard};

    fn temp_file(name: &str) -> ConfigFile {
        ConfigFile::at(fresh_dir(name).join("printer.toml"))
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let loaded = temp_file("config-missing").load(None).unwrap();
        assert_eq!(loaded.saved, Config::default());
        assert_eq!(loaded.saved_profile(), postcard());
        assert_eq!(loaded.profile().unwrap(), postcard());
        assert!(loaded.overrides.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let file = temp_file("config-roundtrip");
        let config = Config {
            fit: Some(Fit::Cover),
            sharpening: Some(Sharpening::Strong),
            background: Some(Background::Black),
            postcard: Some(Profile {
                trim_short_a_mm: 1.8,
                ..postcard()
            }),
        };
        file.save(&config).unwrap();
        assert!(file.exists().unwrap());
        assert_eq!(file.load(None).unwrap().saved, config);
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.starts_with("# selphy printer geometry"), "{text}");
        assert!(text.contains("fit = \"cover\"\n"), "{text}");
        assert!(text.contains("sharpening = \"strong\"\n"), "{text}");
        assert!(text.contains("background = \"black\"\n"), "{text}");
        assert!(text.contains("[postcard]\n"), "{text}");
    }

    #[test]
    fn missing_keys_take_the_built_in_values() {
        let file = temp_file("config-partial");
        fs::write(file.path(), "[postcard]\ntrim_long_a_mm = 4.0\n").unwrap();
        let profile = file.load(None).unwrap().saved_profile();
        assert_eq!(profile.trim_long_a_mm, 4.0);
        assert_eq!(profile.trim_long_b_mm, postcard().trim_long_b_mm);
    }

    #[test]
    fn an_old_flat_file_says_to_move_the_keys_under_postcard() {
        let file = temp_file("config-flat");
        fs::write(file.path(), "trim_long_a_mm = 4.0\n").unwrap();
        let err = file.load(None).unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            format!(
                "{}: the profile keys must be under [postcard]. Move them into a [postcard] \
                 table.",
                file.path().display()
            )
        );
    }

    #[test]
    fn the_fit_is_the_callers_then_the_envs_then_the_files_then_contain() {
        let file = temp_file("config-fit-order");
        let fit = |file: &ConfigFile, given| file.load(given).unwrap().fit;
        assert_eq!(fit(&file, None), Fit::Contain);

        fs::write(file.path(), "fit = \"cover\"\n").unwrap();
        assert_eq!(fit(&file, None), Fit::Cover);

        let with_env = file.clone().with_overrides([(FIT_ENV, "contain")]);
        assert_eq!(fit(&with_env, None), Fit::Contain);
        assert_eq!(fit(&with_env, Some(Fit::Cover)), Fit::Cover);

        let empty_env = file.clone().with_overrides([(FIT_ENV, "")]);
        assert_eq!(fit(&empty_env, None), Fit::Cover);
    }

    #[test]
    fn an_unknown_fit_in_the_env_names_the_variable() {
        let file = temp_file("config-fit-env-bad").with_overrides([(FIT_ENV, "fill")]);
        let err = file.load(None).unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "SELPHY_FIT = fill: unknown fit \"fill\"; the fits are contain, cover"
        );
        assert_eq!(file.load(Some(Fit::Cover)).unwrap().fit, Fit::Cover);
    }

    #[test]
    fn a_paper_key_or_another_papers_table_is_an_error() {
        for (i, text) in ["paper = \"postcard\"\n", "[card]\ntrim_long_a_mm = 1.0\n"]
            .into_iter()
            .enumerate()
        {
            let file = temp_file(&format!("config-paper-gone-{i}"));
            fs::write(file.path(), text).unwrap();
            let err = format!("{:#}", file.load(None).unwrap_err());
            assert!(err.contains("unknown field"), "{text}: {err}");
        }
    }

    #[test]
    fn values_that_break_the_layout_are_rejected_and_name_the_table() {
        let cases = [
            (
                "[postcard]\ncanvas_short_mm = -5.0",
                "[postcard] canvas_short_mm = -5: must be more than 0",
            ),
            (
                "[postcard]\ncanvas_long_mm = nan",
                "[postcard] canvas_long_mm = NaN: must be more than 0",
            ),
            ("[postcard]\ntrim_top_mm = 1.0", "[postcard] unknown field"),
            (
                "[postcard]\ntrim_long_a_mm = -1",
                "[postcard] trim_long_a_mm = -1: must be 0 or more",
            ),
            (
                "[postcard]\nmax_stretch_pct = -200.0",
                "[postcard] max_stretch_pct = -200: must be 0 or more",
            ),
            (
                "[postcard]\ntrim_long_a_mm = 200.0",
                "leaves nothing of the 150 mm canvas_long_mm",
            ),
        ];
        for (i, (text, reason)) in cases.into_iter().enumerate() {
            let file = temp_file(&format!("config-invalid-{i}"));
            fs::write(file.path(), format!("{text}\n")).unwrap();
            let err = file.load(None).unwrap_err();
            assert!(format!("{err:#}").contains(reason), "{text}: {err:#}");
        }
    }

    #[test]
    fn invalid_values_are_not_saved() {
        let file = temp_file("config-save-invalid");
        let config = Config::default().with_profile(Profile {
            trim_short_a_mm: 99.0,
            ..postcard()
        });
        let err = file.save(&config).unwrap_err();
        assert!(err.to_string().starts_with("[postcard] "), "{err:#}");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn missing_output_settings_are_the_defaults_and_with_look_sets_them() {
        let file = temp_file("config-look");
        fs::write(file.path(), "background = \"black\"\n").unwrap();
        let saved = file.load(None).unwrap().saved;
        let look = saved.look();
        assert_eq!(look.sharpening, Sharpening::Standard);
        assert_eq!(look.background, Background::Black);

        let off = Look {
            sharpening: Sharpening::Off,
            ..look
        };
        assert_eq!(saved.with_look(off).look(), off);
        assert_eq!(Config::default().look(), Look::default());
    }

    #[test]
    fn an_unknown_output_setting_value_is_an_error() {
        let file = temp_file("config-look-bad");
        fs::write(file.path(), "sharpening = \"max\"\n").unwrap();
        let err = format!("{:#}", file.load(None).unwrap_err());
        assert!(err.contains("max"), "{err}");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let file = temp_file("config-typo");
        fs::write(file.path(), "fitt = \"cover\"\n").unwrap();
        let err = file.load(None).unwrap_err();
        assert!(format!("{err:#}").contains("fitt"), "{err:#}");
    }

    #[test]
    fn with_profile_sets_the_table_and_keeps_the_fit() {
        let before = Config {
            fit: Some(Fit::Cover),
            ..Config::default()
        };
        let profile = Profile {
            trim_long_a_mm: 1.5,
            ..postcard()
        };
        let after = before.with_profile(profile.clone());
        assert_eq!(after.postcard, Some(profile.clone()));
        assert_eq!(after.fit, Some(Fit::Cover));
        assert_eq!(after.profile(), profile);
        assert_eq!(before.profile(), postcard());
    }

    #[test]
    fn an_override_changes_the_profile_and_not_saved() {
        let file = temp_file("config-override").with_overrides([
            ("SELPHY_TRIM_LONG_A_MM", "3.25"),
            ("SELPHY_MAX_STRETCH_PCT", "0"),
        ]);
        let loaded = file.load(None).unwrap();
        assert_eq!(loaded.saved, Config::default());
        assert_eq!(loaded.saved_profile(), postcard());
        assert_eq!(
            loaded.profile().unwrap(),
            Profile {
                trim_long_a_mm: 3.25,
                max_stretch_pct: 0.0,
                ..postcard()
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
        let loaded = file.load(None).unwrap();
        assert_eq!(loaded.profile().unwrap(), postcard());
        assert!(loaded.overrides.is_empty());
    }

    #[test]
    fn an_override_that_is_not_a_number_names_the_variable() {
        for value in ["abc", "inf", "NaN"] {
            let file =
                temp_file("config-override-nan").with_overrides([("SELPHY_TRIM_LONG_A_MM", value)]);
            let err = file.load(None).unwrap_err();
            assert_eq!(
                format!("{err:#}"),
                format!("SELPHY_TRIM_LONG_A_MM = {value}: not a number")
            );
        }
    }

    #[test]
    fn an_override_that_breaks_the_layout_names_the_variable_and_the_table() {
        let file =
            temp_file("config-override-invalid").with_overrides([("SELPHY_TRIM_LONG_A_MM", "200")]);
        fs::write(file.path(), "[postcard]\ntrim_long_a_mm = 4.0\n").unwrap();
        let loaded = file.load(None).unwrap();
        let err = format!("{:#}", loaded.profile().unwrap_err());
        assert!(
            err.contains("with SELPHY_TRIM_LONG_A_MM set: [postcard] "),
            "{err}"
        );
        assert!(err.contains("leaves nothing of the 150 mm"), "{err}");
    }

    #[test]
    fn save_of_saved_does_not_write_the_override() {
        let file =
            temp_file("config-override-save").with_overrides([("SELPHY_TRIM_LONG_A_MM", "3.25")]);
        let loaded = file.load(None).unwrap();
        let config = loaded.saved.with_profile(loaded.saved_profile());
        file.save(&config).unwrap();
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 4.5"), "{text}");
        assert_eq!(
            ConfigFile::at(file.path())
                .load(None)
                .unwrap()
                .saved_profile(),
            postcard()
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
        let saved: Vec<_> = ["XDG_CONFIG_HOME", "HOME", "SELPHY_TRIM_LONG_A_MM", FIT_ENV]
            .map(|name| (name, std::env::var_os(name)))
            .into();
        // SAFETY: no other test in this process reads or writes these vars.
        unsafe {
            std::env::set_var("HOME", "/home/me");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::set_var("SELPHY_TRIM_LONG_A_MM", "3.5");
            std::env::set_var(FIT_ENV, "cover");
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
        let loaded = explicit.load(None).unwrap();
        assert_eq!(
            loaded.overrides,
            [Override {
                field: &TRIM_LONG_A,
                value: 3.5
            }]
        );
        assert_eq!(loaded.fit, Fit::Cover);
    }
}
