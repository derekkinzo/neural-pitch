//! Persistent user-tunable settings for the NeuralPitch tuner pipeline.
//!
//! `TunerSettings` lives in `neural-pitch-core` (not the Tauri shell) so the
//! same struct round-trips through `tauri-plugin-store` on desktop **and**
//! through future CLI / mobile shells. The Tauri layer holds a cached copy
//! and is responsible for persistence; this module is pure data.
//!
//! # Schema versioning
//!
//! Every persisted blob carries [`TunerSettings::schema_version`]. Hand-edits
//! and version drift are handled by [`migrate`], which dispatches a chain of
//! `migrate_v{N}_to_v{N+1}` functions. The loop shape is fixed; subsequent
//! migrations add new arms to its `match`. See ADR-0013.
//!
//! # Defaults
//!
//! `TunerSettings::default` returns a reference configuration suitable for
//! the YIN/MPM pipeline at 48 kHz with a 2048-sample window and a
//! 512-sample hop. All fields are `#[serde(default)]` (via the struct-level
//! attribute), so a partial JSON blob written by a hand-edit recovers
//! cleanly.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pitch::InstrumentHint;

/// Current persisted-settings schema version.
///
/// Bump this whenever the on-disk shape changes in a way that requires
/// data movement; add a `migrate_v{N}_to_v{N+1}` arm to [`migrate`] in the
/// same change. Reading a v0 blob (i.e. one missing `schema_version`)
/// transparently lifts to the current version through the migration chain.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// Default reference pitch in Hertz (concert A).
pub const DEFAULT_A4_HZ: f32 = 440.0;
/// Default sample rate in Hertz.
pub const DEFAULT_SAMPLE_RATE_HZ: u32 = 48_000;
/// Default analysis window length, in samples.
pub const DEFAULT_WINDOW_SIZE: usize = 2048;
/// Default analysis hop, in samples.
pub const DEFAULT_HOP_SIZE: usize = 512;
/// Default contour smoothing window, in milliseconds.
pub const DEFAULT_SMOOTHING_MS: f32 = 300.0;

/// Inclusive lower bound on the configurable A4 reference (Hz).
pub const A4_HZ_MIN: f32 = 380.0;
/// Inclusive upper bound on the configurable A4 reference (Hz).
pub const A4_HZ_MAX: f32 = 480.0;

/// Persisted, user-tunable tuner settings.
///
/// Held in core (not the Tauri shell) so the same struct serialises through
/// `tauri-plugin-store` on desktop and through future CLI / mobile shells.
/// Every field has `#[serde(default)]` (via the struct-level attribute) so a
/// partial JSON blob written by a hand-edit recovers cleanly. See
/// `docs/design/DESIGN.md` §6 and ADR-0013.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TunerSettings {
    /// On-disk schema version. Lifted by [`migrate`] on load. Always written
    /// as [`SETTINGS_SCHEMA_VERSION`] on save.
    pub schema_version: u32,

    /// Reference pitch in Hertz. Range `[A4_HZ_MIN, A4_HZ_MAX]`. Default
    /// `440.0`.
    pub a4_hz: f32,

    /// Coarse instrument category used to bias estimator priors. See
    /// [`InstrumentHint`].
    pub instrument_hint: InstrumentHint,

    /// Sample rate of the capture stream, in Hertz. Default `48_000`.
    pub sample_rate_hz: u32,

    /// Analysis window length, in samples. MUST be a power of two and
    /// `>= hop_size`. Default `2048`.
    pub window_size: usize,

    /// Analysis hop, in samples. MUST be a power of two and `<= window_size`.
    /// Default `512`.
    pub hop_size: usize,

    /// Contour smoothing window, in milliseconds. Default `300.0`.
    pub smoothing_window_ms: f32,
}

impl Default for TunerSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            a4_hz: DEFAULT_A4_HZ,
            instrument_hint: InstrumentHint::Generic,
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            window_size: DEFAULT_WINDOW_SIZE,
            hop_size: DEFAULT_HOP_SIZE,
            smoothing_window_ms: DEFAULT_SMOOTHING_MS,
        }
    }
}

impl TunerSettings {
    /// Validate the struct as a whole. Returns the first detected error.
    ///
    /// Range invariants:
    ///
    /// - `a4_hz` finite and within `[A4_HZ_MIN, A4_HZ_MAX]`
    /// - `sample_rate_hz` strictly positive
    /// - `window_size` strictly positive, a power of two
    /// - `hop_size` strictly positive, a power of two, and `<= window_size`
    /// - `smoothing_window_ms` finite and non-negative
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !self.a4_hz.is_finite() || self.a4_hz < A4_HZ_MIN || self.a4_hz > A4_HZ_MAX {
            return Err(SettingsError::OutOfRange {
                field: "a4_hz",
                detail: format!("{} not in [{A4_HZ_MIN}, {A4_HZ_MAX}] Hz", self.a4_hz),
            });
        }
        if self.sample_rate_hz == 0 {
            return Err(SettingsError::OutOfRange {
                field: "sample_rate_hz",
                detail: "must be > 0".into(),
            });
        }
        if self.window_size == 0 || !self.window_size.is_power_of_two() {
            return Err(SettingsError::OutOfRange {
                field: "window_size",
                detail: format!("{} must be a power of two and > 0", self.window_size),
            });
        }
        if self.hop_size == 0 || !self.hop_size.is_power_of_two() {
            return Err(SettingsError::OutOfRange {
                field: "hop_size",
                detail: format!("{} must be a power of two and > 0", self.hop_size),
            });
        }
        if self.hop_size > self.window_size {
            return Err(SettingsError::OutOfRange {
                field: "hop_size",
                detail: format!(
                    "{} must be <= window_size ({})",
                    self.hop_size, self.window_size
                ),
            });
        }
        if !self.smoothing_window_ms.is_finite() || self.smoothing_window_ms < 0.0 {
            return Err(SettingsError::OutOfRange {
                field: "smoothing_window_ms",
                detail: format!("{} must be finite and >= 0", self.smoothing_window_ms),
            });
        }
        Ok(())
    }

    /// Apply a single `(key, value)` patch. Returns the new validated
    /// [`TunerSettings`] without mutating `self` on validation failure.
    ///
    /// This is the primitive the Tauri `set_setting` command builds on.
    pub fn with_patch(&self, key: &str, value: Value) -> Result<Self, SettingsError> {
        let mut next = self.clone();
        match key {
            "a4_hz" => {
                next.a4_hz = parse_field("a4_hz", value)?;
            }
            "instrument_hint" => {
                next.instrument_hint = parse_field("instrument_hint", value)?;
            }
            "sample_rate_hz" => {
                next.sample_rate_hz = parse_field("sample_rate_hz", value)?;
            }
            "window_size" => {
                next.window_size = parse_field("window_size", value)?;
            }
            "hop_size" => {
                next.hop_size = parse_field("hop_size", value)?;
            }
            "smoothing_window_ms" => {
                next.smoothing_window_ms = parse_field("smoothing_window_ms", value)?;
            }
            other => {
                return Err(SettingsError::UnknownField(other.to_string()));
            }
        }
        next.schema_version = SETTINGS_SCHEMA_VERSION;
        next.validate()?;
        Ok(next)
    }
}

fn parse_field<T: for<'de> Deserialize<'de>>(
    key: &'static str,
    value: Value,
) -> Result<T, SettingsError> {
    serde_json::from_value::<T>(value).map_err(|e| SettingsError::TypeMismatch {
        field: key,
        detail: e.to_string(),
    })
}

/// Errors produced by [`TunerSettings::validate`] and
/// [`TunerSettings::with_patch`].
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// A field's value was out of its accepted range.
    #[error("{field}: {detail}")]
    OutOfRange {
        /// The field that failed validation.
        field: &'static str,
        /// A human-readable explanation of the failure.
        detail: String,
    },

    /// A patch attempted to set a field whose JSON shape did not match the
    /// expected Rust type.
    #[error("{field}: type mismatch — {detail}")]
    TypeMismatch {
        /// The field that failed deserialisation.
        field: &'static str,
        /// The underlying serde_json error message.
        detail: String,
    },

    /// A patch referenced a field name that does not exist on
    /// [`TunerSettings`].
    #[error("unknown setting field: {0}")]
    UnknownField(String),
}

/// Lift a persisted JSON blob to the current schema version.
///
/// Reads `schema_version` (defaulting to `0` when absent or non-numeric),
/// then iterates the migration chain up to [`SETTINGS_SCHEMA_VERSION`].
/// Always returns a `Value` whose top-level shape can be deserialised into
/// [`TunerSettings`] — when in doubt, `TunerSettings::default` is the
/// fallback after `serde_json::from_value` failure (handled at the call site
/// in the shell).
pub fn migrate(value: Value) -> Value {
    let v = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut cur = value;
    for from in v..u64::from(SETTINGS_SCHEMA_VERSION) {
        cur = match from {
            0 => migrate_v0_to_v1(cur),
            // Future versions add arms here. The loop shape is intentionally
            // fixed so each new migration is a one-line change.
            _ => cur,
        };
    }
    cur
}

/// v0 → v1: scaffolding. v0 is the implicit "no schema_version key" shape;
/// v1 adds `schema_version: 1` and is otherwise structurally compatible.
fn migrate_v0_to_v1(mut value: Value) -> Value {
    if let Value::Object(map) = &mut value {
        map.insert("schema_version".to_string(), Value::from(1_u32));
    }
    value
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_roundtrip() {
        let s = TunerSettings::default();
        let json = serde_json::to_string(&s).expect("serialize");
        let back: TunerSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, back);
    }

    #[test]
    fn default_validates() {
        TunerSettings::default().validate().expect("default valid");
    }

    #[test]
    fn partial_blob_falls_back_to_defaults() {
        // Only `a4_hz` set; every other field MUST recover from `Default`.
        let blob = json!({ "a4_hz": 442.0 });
        let s: TunerSettings = serde_json::from_value(blob).expect("partial");
        assert!((s.a4_hz - 442.0).abs() < 1e-6);
        assert_eq!(s.window_size, DEFAULT_WINDOW_SIZE);
        assert_eq!(s.hop_size, DEFAULT_HOP_SIZE);
    }

    #[test]
    fn migrate_v0_blob_inserts_schema_version() {
        let blob = json!({ "a4_hz": 440.0, "window_size": 2048, "hop_size": 512 });
        let migrated = migrate(blob);
        assert_eq!(
            migrated
                .get("schema_version")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            u64::from(SETTINGS_SCHEMA_VERSION)
        );
    }

    #[test]
    fn migrate_idempotent_at_current_version() {
        let mut s = serde_json::to_value(TunerSettings::default()).expect("to_value");
        let before = s.clone();
        s = migrate(s);
        assert_eq!(s, before);
    }

    #[test]
    fn migrate_non_object_passthrough() {
        // Pathological input: a JSON array. The loop should not panic; we
        // return the value unchanged so the caller's `from_value` reports a
        // clean type error.
        let v = json!([1, 2, 3]);
        let out = migrate(v.clone());
        assert_eq!(out, v);
    }

    #[test]
    fn validate_rejects_a4_below_floor() {
        let s = TunerSettings {
            a4_hz: 379.9,
            ..TunerSettings::default()
        };
        let err = s.validate().expect_err("expected OutOfRange");
        match err {
            SettingsError::OutOfRange { field, .. } => assert_eq!(field, "a4_hz"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_a4_above_ceiling() {
        let s = TunerSettings {
            a4_hz: 480.1,
            ..TunerSettings::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_non_power_of_two_window() {
        let s = TunerSettings {
            window_size: 2000,
            ..TunerSettings::default()
        };
        let err = s.validate().expect_err("expected OutOfRange");
        match err {
            SettingsError::OutOfRange { field, .. } => assert_eq!(field, "window_size"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_hop_larger_than_window() {
        let s = TunerSettings {
            window_size: 512,
            hop_size: 1024,
            ..TunerSettings::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn with_patch_a4_ok() {
        let s = TunerSettings::default();
        let next = s.with_patch("a4_hz", json!(442.0)).expect("patch ok");
        assert!((next.a4_hz - 442.0).abs() < 1e-6);
        // Original is unchanged.
        assert!((s.a4_hz - 440.0).abs() < 1e-6);
    }

    #[test]
    fn with_patch_unknown_field() {
        let s = TunerSettings::default();
        let err = s
            .with_patch("nonexistent", json!(0))
            .expect_err("unknown field");
        match err {
            SettingsError::UnknownField(name) => assert_eq!(name, "nonexistent"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn with_patch_invalid_value_does_not_mutate() {
        let s = TunerSettings::default();
        let err = s
            .with_patch("a4_hz", json!(100.0))
            .expect_err("out of range");
        match err {
            SettingsError::OutOfRange { field, .. } => assert_eq!(field, "a4_hz"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn with_patch_type_mismatch() {
        let s = TunerSettings::default();
        let err = s
            .with_patch("a4_hz", json!("nope"))
            .expect_err("type mismatch");
        match err {
            SettingsError::TypeMismatch { field, .. } => assert_eq!(field, "a4_hz"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
