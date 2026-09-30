//! The one list of `Config` fields: each field's TOML key, its env var, and
//! where it lives in `Config`. The env overrides go over [`FIELDS`], and
//! `Trim` and the GUI's inputs reach their fields through it, so a new field
//! is added here and nowhere else.

use super::Config;
use crate::geometry::Trim;

/// One `f64` field of [`Config`].
#[derive(Debug)]
pub struct Field {
    /// The TOML key, which is also the field's name in `Config`.
    pub key: &'static str,
    /// The env var that overrides the field: `SELPHY_` and the key in upper
    /// case.
    pub env: &'static str,
    get: fn(&Config) -> f64,
    get_mut: fn(&mut Config) -> &mut f64,
}

impl Field {
    /// The field's value in `cfg`.
    pub fn get(&self, cfg: &Config) -> f64 {
        (self.get)(cfg)
    }

    /// The field in `cfg`, for writing.
    pub fn get_mut<'a>(&self, cfg: &'a mut Config) -> &'a mut f64 {
        (self.get_mut)(cfg)
    }

    /// The field that holds `trim`.
    pub fn for_trim(trim: Trim) -> &'static Field {
        match trim {
            Trim::LongA => &TRIM_LONG_A,
            Trim::LongB => &TRIM_LONG_B,
            Trim::ShortA => &TRIM_SHORT_A,
            Trim::ShortB => &TRIM_SHORT_B,
        }
    }
}

/// Fields are equal when their keys are.
impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

/// `canvas_long_mm`.
pub const CANVAS_LONG: Field = Field {
    key: "canvas_long_mm",
    env: "SELPHY_CANVAS_LONG_MM",
    get: |c| c.canvas_long_mm,
    get_mut: |c| &mut c.canvas_long_mm,
};
/// `canvas_short_mm`.
pub const CANVAS_SHORT: Field = Field {
    key: "canvas_short_mm",
    env: "SELPHY_CANVAS_SHORT_MM",
    get: |c| c.canvas_short_mm,
    get_mut: |c| &mut c.canvas_short_mm,
};
/// `trim_long_a_mm`.
pub const TRIM_LONG_A: Field = Field {
    key: "trim_long_a_mm",
    env: "SELPHY_TRIM_LONG_A_MM",
    get: |c| c.trim_long_a_mm,
    get_mut: |c| &mut c.trim_long_a_mm,
};
/// `trim_long_b_mm`.
pub const TRIM_LONG_B: Field = Field {
    key: "trim_long_b_mm",
    env: "SELPHY_TRIM_LONG_B_MM",
    get: |c| c.trim_long_b_mm,
    get_mut: |c| &mut c.trim_long_b_mm,
};
/// `trim_short_a_mm`.
pub const TRIM_SHORT_A: Field = Field {
    key: "trim_short_a_mm",
    env: "SELPHY_TRIM_SHORT_A_MM",
    get: |c| c.trim_short_a_mm,
    get_mut: |c| &mut c.trim_short_a_mm,
};
/// `trim_short_b_mm`.
pub const TRIM_SHORT_B: Field = Field {
    key: "trim_short_b_mm",
    env: "SELPHY_TRIM_SHORT_B_MM",
    get: |c| c.trim_short_b_mm,
    get_mut: |c| &mut c.trim_short_b_mm,
};
/// `max_stretch_pct`.
pub const MAX_STRETCH: Field = Field {
    key: "max_stretch_pct",
    env: "SELPHY_MAX_STRETCH_PCT",
    get: |c| c.max_stretch_pct,
    get_mut: |c| &mut c.max_stretch_pct,
};

/// Every field, in the order of `Config`.
pub const FIELDS: [&Field; 7] = [
    &CANVAS_LONG,
    &CANVAS_SHORT,
    &TRIM_LONG_A,
    &TRIM_LONG_B,
    &TRIM_SHORT_A,
    &TRIM_SHORT_B,
    &MAX_STRETCH,
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_config_field_has_one_entry_and_a_unique_env_var() {
        let text = toml::to_string(&Config::default()).unwrap();
        let table: toml::Table = toml::from_str(&text).unwrap();
        let keys: BTreeSet<&str> = table.keys().map(String::as_str).collect();
        let listed: BTreeSet<&str> = FIELDS.iter().map(|f| f.key).collect();
        assert_eq!(listed, keys);
        assert_eq!(listed.len(), FIELDS.len(), "a key is listed twice");

        let envs: BTreeSet<&str> = FIELDS.iter().map(|f| f.env).collect();
        assert_eq!(envs.len(), FIELDS.len(), "an env var is listed twice");
        for field in FIELDS {
            assert_eq!(
                field.env,
                format!("SELPHY_{}", field.key.to_uppercase()),
                "{}",
                field.key
            );
        }
    }

    #[test]
    fn each_field_reads_and_writes_its_key() {
        for (i, field) in FIELDS.into_iter().enumerate() {
            let mut cfg = Config::default();
            let value = 100.0 + i as f64;
            *field.get_mut(&mut cfg) = value;
            assert_eq!(field.get(&cfg), value);
            let table: toml::Table = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
            assert_eq!(table[field.key].as_float(), Some(value), "{}", field.key);
        }
    }

    #[test]
    fn each_trim_has_its_field() {
        let keys = Trim::ALL.map(|trim| Field::for_trim(trim).key);
        assert_eq!(
            keys,
            [
                "trim_long_a_mm",
                "trim_long_b_mm",
                "trim_short_a_mm",
                "trim_short_b_mm"
            ]
        );
    }
}
