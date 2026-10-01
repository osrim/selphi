//! The one list of `Profile` fields, with each field's TOML key and env var.
//! The env overrides, `Trim` and the GUI's inputs all go through [`FIELDS`],
//! so a new field is added here and nowhere else.

use super::Profile;
use crate::geometry::Trim;

/// One `f64` field of [`Profile`].
#[derive(Debug)]
pub struct Field {
    /// The TOML key, which is also the field's name in `Profile`.
    pub key: &'static str,
    /// The env var that overrides the field: `SELPHI_` and the key in upper
    /// case.
    pub env: &'static str,
    get: fn(&Profile) -> f64,
    get_mut: fn(&mut Profile) -> &mut f64,
}

impl Field {
    /// The field's value in `profile`.
    pub fn get(&self, profile: &Profile) -> f64 {
        (self.get)(profile)
    }

    /// The field in `profile`, for writing.
    pub fn get_mut<'a>(&self, profile: &'a mut Profile) -> &'a mut f64 {
        (self.get_mut)(profile)
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

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

/// `canvas_long_mm`.
pub const CANVAS_LONG: Field = Field {
    key: "canvas_long_mm",
    env: "SELPHI_CANVAS_LONG_MM",
    get: |c| c.canvas_long_mm,
    get_mut: |c| &mut c.canvas_long_mm,
};
/// `canvas_short_mm`.
pub const CANVAS_SHORT: Field = Field {
    key: "canvas_short_mm",
    env: "SELPHI_CANVAS_SHORT_MM",
    get: |c| c.canvas_short_mm,
    get_mut: |c| &mut c.canvas_short_mm,
};
/// `trim_long_a_mm`.
pub const TRIM_LONG_A: Field = Field {
    key: "trim_long_a_mm",
    env: "SELPHI_TRIM_LONG_A_MM",
    get: |c| c.trim_long_a_mm,
    get_mut: |c| &mut c.trim_long_a_mm,
};
/// `trim_long_b_mm`.
pub const TRIM_LONG_B: Field = Field {
    key: "trim_long_b_mm",
    env: "SELPHI_TRIM_LONG_B_MM",
    get: |c| c.trim_long_b_mm,
    get_mut: |c| &mut c.trim_long_b_mm,
};
/// `trim_short_a_mm`.
pub const TRIM_SHORT_A: Field = Field {
    key: "trim_short_a_mm",
    env: "SELPHI_TRIM_SHORT_A_MM",
    get: |c| c.trim_short_a_mm,
    get_mut: |c| &mut c.trim_short_a_mm,
};
/// `trim_short_b_mm`.
pub const TRIM_SHORT_B: Field = Field {
    key: "trim_short_b_mm",
    env: "SELPHI_TRIM_SHORT_B_MM",
    get: |c| c.trim_short_b_mm,
    get_mut: |c| &mut c.trim_short_b_mm,
};
/// `max_stretch_pct`.
pub const MAX_STRETCH: Field = Field {
    key: "max_stretch_pct",
    env: "SELPHI_MAX_STRETCH_PCT",
    get: |c| c.max_stretch_pct,
    get_mut: |c| &mut c.max_stretch_pct,
};

/// Every field, in the order of `Profile`.
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
    use crate::test_util::postcard;
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_profile_field_has_one_entry_and_a_unique_env_var() {
        let text = toml::to_string(&postcard()).unwrap();
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
                format!("SELPHI_{}", field.key.to_uppercase()),
                "{}",
                field.key
            );
        }
    }

    #[test]
    fn each_field_reads_and_writes_its_key() {
        for (i, field) in FIELDS.into_iter().enumerate() {
            let mut profile = postcard();
            let value = 100.0 + i as f64;
            *field.get_mut(&mut profile) = value;
            assert_eq!(field.get(&profile), value);
            let table: toml::Table = toml::from_str(&toml::to_string(&profile).unwrap()).unwrap();
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
