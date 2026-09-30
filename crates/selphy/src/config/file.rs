//! The printer config file, and the env vars that override its values for
//! one run. The paper resolves in this order: the caller's value (the
//! command line), then `SELPHY_PAPER`, then the file, then postcard. The fit
//! resolves in the same way, through `SELPHY_FIT`, to contain. Each
//! profile value resolves as env, then file, then defaults. An override is
//! never written to the file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};

use super::fields::{FIELDS, Field};
use super::{Config, Profile};
use crate::geometry::Fit;
use crate::paper::Paper;
use crate::toml_file;

/// The env var that names the paper.
pub const PAPER_ENV: &str = "SELPHY_PAPER";

/// The env var that names the fit.
pub const FIT_ENV: &str = "SELPHY_FIT";

const HEADER: &str = "\
# selphy printer geometry: the default paper and fit, and one table per
# calibrated paper: [postcard], [l] or [card]. selphy rewrites this file, so comments
# added by hand are not kept.
# Trims are mm of canvas lost per edge, named for the landscape canvas:
# long A = left, long B = right, short A = top, short B = bottom.

";

/// The printer config file, and the env overrides for this run.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: PathBuf,
    /// The raw values of the env vars that are set. `load` parses them.
    env_values: Vec<(&'static Field, OsString)>,
    /// The raw value of `SELPHY_PAPER`, when it is set.
    env_paper: Option<OsString>,
    /// The raw value of `SELPHY_FIT`, when it is set.
    env_fit: Option<OsString>,
}

/// The config read from the file, the paper and the fit this run uses, and
/// the overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The file's values. Start from this to edit the file.
    pub saved: Config,
    /// The paper this run uses.
    pub paper: Paper,
    /// The fit this run uses.
    pub fit: Fit,
    /// The overrides that are set, in the order of the field table. They
    /// apply to the profile of `paper`.
    pub overrides: Vec<Override>,
    /// The file's path, for the error messages.
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
    /// `~/.config/selphy/printer.toml`. Reads the env overrides,
    /// `SELPHY_PAPER` and `SELPHY_FIT` once, now.
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
            env_paper: std::env::var_os(PAPER_ENV),
            env_fit: std::env::var_os(FIT_ENV),
        }
    }

    /// The config file at `path`, with no overrides.
    pub fn at(path: impl Into<PathBuf>) -> ConfigFile {
        ConfigFile {
            path: path.into(),
            env_values: Vec::new(),
            env_paper: None,
            env_fit: None,
        }
    }

    /// This file with the env vars in `pairs`: names and values, as if read
    /// from the environment. A name is `SELPHY_PAPER`, `SELPHY_FIT` or the
    /// env var of a field.
    ///
    /// # Panics
    ///
    /// If a name is neither.
    pub fn with_overrides<'a>(
        mut self,
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        for (name, value) in pairs {
            if name == PAPER_ENV {
                self.env_paper = Some(value.into());
                continue;
            }
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

    /// Reads and checks the file, parses the overrides, and resolves the
    /// paper: `paper` if given, else `SELPHY_PAPER`, else the file's
    /// `paper`, else postcard. The fit resolves in the same way: `fit`, else
    /// `SELPHY_FIT`, else the file's `fit`, else contain. A missing file
    /// gives the defaults.
    ///
    /// An old file with the profile keys at the top level is an error that
    /// says to move them into a `[postcard]` table.
    pub fn load(&self, paper: Option<Paper>, fit: Option<Fit>) -> Result<Loaded> {
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
        let paper = match paper {
            Some(paper) => paper,
            None => parse_env(PAPER_ENV, self.env_paper.as_ref())?
                .or(saved.paper)
                .unwrap_or(Paper::Postcard),
        };
        let fit = match fit {
            Some(fit) => fit,
            None => parse_env(FIT_ENV, self.env_fit.as_ref())?
                .or(saved.fit)
                .unwrap_or(Fit::Contain),
        };
        Ok(Loaded {
            saved,
            paper,
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
    /// The file's profile of the paper, without the overrides. Start from
    /// this to edit the paper's table. An uncalibrated paper is an error
    /// that names the command to calibrate it.
    pub fn saved_profile(&self) -> Result<Profile> {
        let paper = self.paper;
        self.saved.profile(paper).ok_or_else(|| {
            anyhow!("{paper} paper is not calibrated. Run: selphy calibrate --paper {paper}")
        })
    }

    /// The profile this run uses: [`Loaded::saved_profile`] with the
    /// overrides applied. Use this to prepare photos.
    pub fn profile(&self) -> Result<Profile> {
        self.apply_overrides(self.saved_profile()?)
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
                self.paper
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
    use crate::test_util::{fresh_dir, postcard};

    /// The `[l]` table of [`l_profile`]: an uncalibrated paper's table
    /// needs all four trims.
    const L_TABLE: &str = "[l]\ntrim_long_a_mm = 1.5\ntrim_long_b_mm = 0.0\n\
                           trim_short_a_mm = 0.0\ntrim_short_b_mm = 0.0\n";

    fn temp_file(name: &str) -> ConfigFile {
        ConfigFile::at(fresh_dir(name).join("printer.toml"))
    }

    fn l_profile() -> Profile {
        Profile {
            trim_long_a_mm: 1.5,
            ..Paper::L.starting_profile()
        }
    }

    #[test]
    fn a_missing_file_gives_postcard_and_its_defaults() {
        let loaded = temp_file("config-missing").load(None, None).unwrap();
        assert_eq!(loaded.saved, Config::default());
        assert_eq!(loaded.paper, Paper::Postcard);
        assert_eq!(loaded.saved_profile().unwrap(), postcard());
        assert_eq!(loaded.profile().unwrap(), postcard());
        assert!(loaded.overrides.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let file = temp_file("config-roundtrip");
        let config = Config {
            paper: Some(Paper::L),
            fit: Some(Fit::Cover),
            postcard: Some(Profile {
                trim_short_a_mm: 1.8,
                ..postcard()
            }),
            l: Some(l_profile()),
            card: None,
        };
        file.save(&config).unwrap();
        assert!(file.exists().unwrap());
        assert_eq!(file.load(None, None).unwrap().saved, config);
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.starts_with("# selphy printer geometry"), "{text}");
        assert!(text.contains("paper = \"l\"\nfit = \"cover\"\n"), "{text}");
        assert!(text.contains("[postcard]\n"), "{text}");
        assert!(text.contains("[l]\n"), "{text}");
        assert!(!text.contains("\n[card]\n"), "{text}");
    }

    #[test]
    fn an_l_table_alone_calibrates_l_and_leaves_postcard_at_its_defaults() {
        let file = temp_file("config-l-only");
        fs::write(file.path(), L_TABLE).unwrap();
        let saved = file.load(None, None).unwrap().saved;
        assert_eq!(saved.profile(Paper::L), Some(l_profile()));
        assert_eq!(saved.profile(Paper::Postcard), Some(postcard()));
        assert_eq!(saved.profile(Paper::Card), None);
    }

    #[test]
    fn missing_keys_take_the_papers_starting_values() {
        let file = temp_file("config-partial");
        fs::write(
            file.path(),
            "[postcard]\ntrim_long_a_mm = 4.0\n[card]\nmax_stretch_pct = 1.0\n\
             trim_long_a_mm = 0.0\ntrim_long_b_mm = 0.0\ntrim_short_a_mm = 0.0\n\
             trim_short_b_mm = 0.0\n",
        )
        .unwrap();
        let saved = file.load(None, None).unwrap().saved;
        let postcard_profile = saved.profile(Paper::Postcard).unwrap();
        assert_eq!(postcard_profile.trim_long_a_mm, 4.0);
        assert_eq!(postcard_profile.trim_long_b_mm, postcard().trim_long_b_mm);
        assert_eq!(
            saved.profile(Paper::Card).unwrap(),
            Profile {
                max_stretch_pct: 1.0,
                ..Paper::Card.starting_profile()
            }
        );
    }

    #[test]
    fn a_table_of_an_uncalibrated_paper_needs_all_four_trims() {
        for (paper, table) in [(Paper::L, "[l]\n"), (Paper::Card, "[card]\n")] {
            let file = temp_file(&format!("config-missing-trims-{paper}"));
            fs::write(file.path(), format!("{table}trim_long_a_mm = 1.5\n")).unwrap();
            let err = format!("{:#}", file.load(None, None).unwrap_err());
            assert!(
                err.contains(&format!(
                    "[{paper}] trim_long_b_mm is missing: {paper} paper has no built-in trims. \
                     Run: selphy calibrate --paper {paper}"
                )),
                "{err}"
            );
        }
        // Postcard's missing trims take its built-in values.
        let file = temp_file("config-missing-trims-postcard");
        fs::write(file.path(), "[postcard]\ntrim_long_a_mm = 1.5\n").unwrap();
        assert!(file.load(None, None).is_ok());
    }

    #[test]
    fn an_old_flat_file_says_to_move_the_keys_under_postcard() {
        let file = temp_file("config-flat");
        fs::write(file.path(), "trim_long_a_mm = 4.0\n").unwrap();
        let err = file.load(None, None).unwrap_err();
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
    fn the_paper_is_the_callers_then_the_envs_then_the_files_then_postcard() {
        let file = temp_file("config-paper-order");
        let paper = |file: &ConfigFile, given| file.load(given, None).unwrap().paper;
        assert_eq!(paper(&file, None), Paper::Postcard);

        fs::write(file.path(), "paper = \"l\"\n").unwrap();
        assert_eq!(paper(&file, None), Paper::L);

        let with_env = file.clone().with_overrides([(PAPER_ENV, "card")]);
        assert_eq!(paper(&with_env, None), Paper::Card);
        assert_eq!(paper(&with_env, Some(Paper::Postcard)), Paper::Postcard);

        let empty_env = file.clone().with_overrides([(PAPER_ENV, "")]);
        assert_eq!(paper(&empty_env, None), Paper::L);
    }

    #[test]
    fn the_fit_is_the_callers_then_the_envs_then_the_files_then_contain() {
        let file = temp_file("config-fit-order");
        let fit = |file: &ConfigFile, given| file.load(None, given).unwrap().fit;
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
        let err = file.load(None, None).unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "SELPHY_FIT = fill: unknown fit \"fill\"; the fits are contain, cover"
        );
        assert_eq!(file.load(None, Some(Fit::Cover)).unwrap().fit, Fit::Cover);
    }

    #[test]
    fn an_unknown_paper_in_the_env_names_the_variable() {
        let file = temp_file("config-paper-env-bad").with_overrides([(PAPER_ENV, "a4")]);
        let err = file.load(None, None).unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "SELPHY_PAPER = a4: unknown paper \"a4\"; the papers are postcard, l, card"
        );
        // A paper from the caller wins, so the env var is not read.
        assert_eq!(file.load(Some(Paper::L), None).unwrap().paper, Paper::L);
    }

    #[test]
    fn an_unknown_paper_in_the_file_is_an_error() {
        let file = temp_file("config-paper-file-bad");
        fs::write(file.path(), "paper = \"a4\"\n").unwrap();
        let err = format!("{:#}", file.load(None, None).unwrap_err());
        assert!(err.starts_with("parsing "), "{err}");
        assert!(err.contains("a4"), "{err}");
    }

    #[test]
    fn an_uncalibrated_paper_names_the_calibrate_command() {
        let loaded = temp_file("config-uncalibrated")
            .load(Some(Paper::Card), None)
            .unwrap();
        for err in [
            loaded.saved_profile().unwrap_err(),
            loaded.profile().unwrap_err(),
        ] {
            assert_eq!(
                err.to_string(),
                "card paper is not calibrated. Run: selphy calibrate --paper card"
            );
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
                "[l]\ntrim_long_a_mm = -1\ntrim_long_b_mm = 0\ntrim_short_a_mm = 0\n\
                 trim_short_b_mm = 0",
                "[l] trim_long_a_mm = -1: must be 0 or more",
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
            let err = file.load(None, None).unwrap_err();
            assert!(format!("{err:#}").contains(reason), "{text}: {err:#}");
        }
    }

    #[test]
    fn invalid_values_are_not_saved() {
        let file = temp_file("config-save-invalid");
        let config = Config::default().with_profile(
            Paper::L,
            Profile {
                trim_short_a_mm: 99.0,
                ..l_profile()
            },
        );
        let err = file.save(&config).unwrap_err();
        assert!(err.to_string().starts_with("[l] "), "{err:#}");
        assert!(!file.exists().unwrap());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let file = temp_file("config-typo");
        fs::write(file.path(), "papr = \"l\"\n").unwrap();
        let err = file.load(None, None).unwrap_err();
        assert!(format!("{err:#}").contains("papr"), "{err:#}");
    }

    #[test]
    fn with_profile_changes_one_table_and_keeps_the_others() {
        let before = Config {
            paper: Some(Paper::Card),
            fit: None,
            postcard: Some(postcard()),
            l: None,
            card: Some(Paper::Card.starting_profile()),
        };
        let after = before.with_profile(Paper::L, l_profile());
        assert_eq!(after.l, Some(l_profile()));
        assert_eq!(
            Config {
                l: None,
                ..after.clone()
            },
            before
        );
        let replaced = after.with_profile(Paper::Card, postcard());
        assert_eq!(replaced.card, Some(postcard()));
        assert_eq!(replaced.l, Some(l_profile()));
    }

    #[test]
    fn an_override_changes_the_profile_and_not_saved() {
        let file = temp_file("config-override").with_overrides([
            ("SELPHY_TRIM_LONG_A_MM", "3.25"),
            ("SELPHY_MAX_STRETCH_PCT", "0"),
        ]);
        let loaded = file.load(None, None).unwrap();
        assert_eq!(loaded.saved, Config::default());
        assert_eq!(loaded.saved_profile().unwrap(), postcard());
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
    fn an_override_applies_to_the_chosen_papers_profile() {
        let file = temp_file("config-override-l").with_overrides([("SELPHY_MAX_STRETCH_PCT", "0")]);
        fs::write(file.path(), L_TABLE).unwrap();
        let loaded = file.load(Some(Paper::L), None).unwrap();
        assert_eq!(
            loaded.profile().unwrap(),
            Profile {
                max_stretch_pct: 0.0,
                ..l_profile()
            }
        );
    }

    #[test]
    fn an_empty_override_is_ignored() {
        let file =
            temp_file("config-override-empty").with_overrides([("SELPHY_TRIM_LONG_A_MM", "")]);
        let loaded = file.load(None, None).unwrap();
        assert_eq!(loaded.profile().unwrap(), postcard());
        assert!(loaded.overrides.is_empty());
    }

    #[test]
    fn an_override_that_is_not_a_number_names_the_variable() {
        for value in ["abc", "inf", "NaN"] {
            let file =
                temp_file("config-override-nan").with_overrides([("SELPHY_TRIM_LONG_A_MM", value)]);
            let err = file.load(None, None).unwrap_err();
            assert_eq!(
                format!("{err:#}"),
                format!("SELPHY_TRIM_LONG_A_MM = {value}: not a number")
            );
        }
    }

    #[test]
    fn an_override_that_breaks_the_layout_names_the_variable_and_the_paper() {
        let file =
            temp_file("config-override-invalid").with_overrides([("SELPHY_TRIM_LONG_A_MM", "200")]);
        fs::write(file.path(), "[postcard]\ntrim_long_a_mm = 4.0\n").unwrap();
        let loaded = file.load(None, None).unwrap();
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
        let loaded = file.load(None, None).unwrap();
        let config = loaded
            .saved
            .with_profile(loaded.paper, loaded.saved_profile().unwrap());
        file.save(&config).unwrap();
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("trim_long_a_mm = 4.5"), "{text}");
        assert_eq!(
            ConfigFile::at(file.path())
                .load(None, None)
                .unwrap()
                .saved_profile()
                .unwrap(),
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
        let saved: Vec<_> = [
            "XDG_CONFIG_HOME",
            "HOME",
            "SELPHY_TRIM_LONG_A_MM",
            PAPER_ENV,
            FIT_ENV,
        ]
        .map(|name| (name, std::env::var_os(name)))
        .into();
        // SAFETY: no other test in this process reads or writes these vars.
        unsafe {
            std::env::set_var("HOME", "/home/me");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::set_var("SELPHY_TRIM_LONG_A_MM", "3.5");
            std::env::set_var(PAPER_ENV, "l");
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
        let loaded = explicit.load(None, None).unwrap();
        assert_eq!(
            loaded.overrides,
            [Override {
                field: &TRIM_LONG_A,
                value: 3.5
            }]
        );
        assert_eq!(loaded.paper, Paper::L);
        assert_eq!(loaded.fit, Fit::Cover);
    }
}
