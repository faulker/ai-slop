//! Persistent gameplay tuning (sim numbers, spawn weights, tint palette).
//!
//! Stored at [`DEFAULT_TUNING_PATH`]. Missing or partial JSON fills defaults
//! so a machine with no override file matches stock `SimConfig` / `GenConfig`.

use crate::course::GenConfig;
use crate::level::SimConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Default relative path for tuning on disk.
pub const DEFAULT_TUNING_PATH: &str = "config/tuning.json";

/// RGB triplet 0–255 for one seed tint slot.
pub type TintRgb = [u8; 3];

/// Editor-tunable numbers plus spawn weights and the 8 tint colors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    pub max_speed: f32,
    pub accel: f32,
    pub brake: f32,
    pub debris_damage: f32,
    pub contact_damage: f32,
    pub pipe_damage: f32,
    pub laser_damage: f32,
    pub boost_duration: f32,
    pub debris_weight: f32,
    pub pill_weight: f32,
    pub gang_weight: f32,
    /// Eight seed palette colors as RGB 0–255 (index matches entity `color`).
    pub tints: [TintRgb; 8],
}

impl Default for Tuning {
    fn default() -> Self {
        let sim = SimConfig::default();
        let gen = GenConfig::default();
        Self {
            max_speed: sim.max_speed,
            accel: sim.accel,
            brake: sim.brake,
            debris_damage: sim.debris_damage,
            contact_damage: sim.contact_damage,
            pipe_damage: sim.pipe_damage,
            laser_damage: sim.laser_damage,
            boost_duration: sim.boost_duration,
            debris_weight: gen.debris_weight,
            pill_weight: gen.pill_weight,
            gang_weight: gen.gang_weight,
            tints: default_tints(),
        }
    }
}

/// Default tint table matching the historical multiplier palette (as RGB).
pub fn default_tints() -> [TintRgb; 8] {
    [
        [255, 255, 255], // 0 canonical
        [255, 140, 140], // 1 red   (1.00, 0.55, 0.55)
        [140, 217, 255], // 2 blue  (0.55, 0.85, 1.00)
        [140, 255, 179], // 3 green (0.55, 1.00, 0.70)
        [255, 217, 102], // 4 amber (1.00, 0.85, 0.40)
        [217, 140, 255], // 5 violet(0.85, 0.55, 1.00)
        [115, 255, 255], // 6 cyan  (0.45, 1.00, 1.00)
        [255, 179, 102], // 7 orange(1.00, 0.70, 0.40)
    ]
}

/// Errors from loading or saving tuning.
#[derive(Debug)]
pub enum TuningError {
    Io(io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for TuningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TuningError::Io(e) => write!(f, "tuning io error: {e}"),
            TuningError::Parse(e) => write!(f, "tuning parse error: {e}"),
        }
    }
}

impl std::error::Error for TuningError {}

impl From<io::Error> for TuningError {
    fn from(value: io::Error) -> Self {
        TuningError::Io(value)
    }
}

impl From<serde_json::Error> for TuningError {
    fn from(value: serde_json::Error) -> Self {
        TuningError::Parse(value)
    }
}

impl Tuning {
    /// Builds a [`SimConfig`] from tuned fields; untuned fields stay at defaults.
    pub fn to_sim_config(&self) -> SimConfig {
        SimConfig {
            max_speed: self.max_speed,
            accel: self.accel,
            brake: self.brake,
            debris_damage: self.debris_damage,
            contact_damage: self.contact_damage,
            pipe_damage: self.pipe_damage,
            laser_damage: self.laser_damage,
            boost_duration: self.boost_duration,
            ..SimConfig::default()
        }
    }

    /// Builds a [`GenConfig`] from the three spawn weights.
    pub fn to_gen_config(&self) -> GenConfig {
        GenConfig {
            debris_weight: self.debris_weight,
            pill_weight: self.pill_weight,
            gang_weight: self.gang_weight,
            ..GenConfig::default()
        }
    }

    /// Serializes tuning to pretty JSON.
    pub fn to_json_string(&self) -> Result<String, TuningError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses tuning, filling any missing fields from defaults. Never panics.
    pub fn from_json_lenient(json: &str) -> Result<Self, TuningError> {
        #[derive(Deserialize)]
        struct Partial {
            max_speed: Option<f32>,
            accel: Option<f32>,
            brake: Option<f32>,
            debris_damage: Option<f32>,
            contact_damage: Option<f32>,
            pipe_damage: Option<f32>,
            laser_damage: Option<f32>,
            boost_duration: Option<f32>,
            debris_weight: Option<f32>,
            pill_weight: Option<f32>,
            gang_weight: Option<f32>,
            tints: Option<Vec<Vec<u8>>>,
        }
        let partial: Partial = serde_json::from_str(json)?;
        let defaults = Self::default();
        let tints = match partial.tints {
            Some(rows) => merge_tints(&defaults.tints, &rows),
            None => defaults.tints,
        };
        Ok(Self {
            max_speed: partial.max_speed.unwrap_or(defaults.max_speed),
            accel: partial.accel.unwrap_or(defaults.accel),
            brake: partial.brake.unwrap_or(defaults.brake),
            debris_damage: partial.debris_damage.unwrap_or(defaults.debris_damage),
            contact_damage: partial.contact_damage.unwrap_or(defaults.contact_damage),
            pipe_damage: partial.pipe_damage.unwrap_or(defaults.pipe_damage),
            laser_damage: partial.laser_damage.unwrap_or(defaults.laser_damage),
            boost_duration: partial.boost_duration.unwrap_or(defaults.boost_duration),
            debris_weight: partial.debris_weight.unwrap_or(defaults.debris_weight),
            pill_weight: partial.pill_weight.unwrap_or(defaults.pill_weight),
            gang_weight: partial.gang_weight.unwrap_or(defaults.gang_weight),
            tints,
        })
    }

    /// Loads from `path`, or returns defaults if the file is missing (does not write).
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self, TuningError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        Self::from_json_lenient(&text)
    }

    /// Writes tuning as pretty JSON, creating parent directories as needed.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), TuningError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_json_string()?)?;
        Ok(())
    }

    /// Default config path relative to the process working directory.
    pub fn default_path() -> PathBuf {
        PathBuf::from(DEFAULT_TUNING_PATH)
    }
}

/// Merges a partial tint list into the default 8-entry table.
fn merge_tints(defaults: &[TintRgb; 8], rows: &[Vec<u8>]) -> [TintRgb; 8] {
    let mut out = *defaults;
    for (i, row) in rows.iter().enumerate().take(8) {
        if row.len() >= 3 {
            out[i] = [row[0], row[1], row[2]];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuning_json_roundtrip() {
        let mut t = Tuning::default();
        t.max_speed = 55.0;
        t.debris_weight = 2.0;
        t.tints[1] = [10, 20, 30];
        let json = t.to_json_string().expect("serialize");
        let loaded = Tuning::from_json_lenient(&json).expect("parse");
        assert_eq!(t, loaded);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path = std::env::temp_dir().join(format!(
            "bike-tuning-missing-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_file(&path);
        let t = Tuning::load_or_default(&path).expect("load");
        assert_eq!(t, Tuning::default());
        assert_eq!(t.to_sim_config().max_speed, SimConfig::default().max_speed);
        assert_eq!(t.to_gen_config(), GenConfig::default());
    }

    #[test]
    fn partial_json_fills_defaults_no_panic() {
        let json = r#"{ "max_speed": 99.0, "tints": [[1,2,3]] }"#;
        let t = Tuning::from_json_lenient(json).expect("parse");
        assert!((t.max_speed - 99.0).abs() < 1e-5);
        assert_eq!(t.tints[0], [1, 2, 3]);
        assert_eq!(t.tints[1], default_tints()[1]);
        assert!((t.accel - SimConfig::default().accel).abs() < 1e-5);
        assert!((t.debris_weight - 1.0).abs() < 1e-5);
    }

    #[test]
    fn empty_object_is_all_defaults() {
        let t = Tuning::from_json_lenient("{}").expect("parse");
        assert_eq!(t, Tuning::default());
    }

    #[test]
    fn default_tuning_matches_sim_and_gen_defaults() {
        let t = Tuning::default();
        let sim = t.to_sim_config();
        let d = SimConfig::default();
        assert!((sim.max_speed - d.max_speed).abs() < 1e-5);
        assert!((sim.accel - d.accel).abs() < 1e-5);
        assert!((sim.brake - d.brake).abs() < 1e-5);
        assert!((sim.debris_damage - d.debris_damage).abs() < 1e-5);
        assert!((sim.contact_damage - d.contact_damage).abs() < 1e-5);
        assert!((sim.pipe_damage - d.pipe_damage).abs() < 1e-5);
        assert!((sim.laser_damage - d.laser_damage).abs() < 1e-5);
        assert!((sim.boost_duration - d.boost_duration).abs() < 1e-5);
        assert_eq!(t.to_gen_config(), GenConfig::default());
    }
}
