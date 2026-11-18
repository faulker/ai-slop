//! Customizable key bindings persisted as JSON.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Default relative path for bindings on disk.
pub const DEFAULT_BINDINGS_PATH: &str = "config/keys.json";

/// Logical player actions that can be bound to keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Accelerate,
    Brake,
    SteerLeft,
    SteerRight,
    PipeLeft,
    PipeRight,
    Laser,
}

impl Action {
    /// All actions in UI list order.
    pub fn all() -> &'static [Action] {
        &[
            Action::Accelerate,
            Action::Brake,
            Action::SteerLeft,
            Action::SteerRight,
            Action::PipeLeft,
            Action::PipeRight,
            Action::Laser,
        ]
    }

    /// Short label for menus.
    pub fn label(self) -> &'static str {
        match self {
            Action::Accelerate => "Accelerate",
            Action::Brake => "Brake",
            Action::SteerLeft => "Steer Left",
            Action::SteerRight => "Steer Right",
            Action::PipeLeft => "Pipe Left",
            Action::PipeRight => "Pipe Right",
            Action::Laser => "Laser",
        }
    }
}

/// Errors from loading or saving bindings.
#[derive(Debug)]
pub enum BindingsError {
    Io(io::Error),
    Parse(serde_json::Error),
}

impl std::fmt::Display for BindingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindingsError::Io(e) => write!(f, "bindings io error: {e}"),
            BindingsError::Parse(e) => write!(f, "bindings parse error: {e}"),
        }
    }
}

impl std::error::Error for BindingsError {}

impl From<io::Error> for BindingsError {
    fn from(value: io::Error) -> Self {
        BindingsError::Io(value)
    }
}

impl From<serde_json::Error> for BindingsError {
    fn from(value: serde_json::Error) -> Self {
        BindingsError::Parse(value)
    }
}

/// A named key code stored as a string (stable across platforms for our defaults).
///
/// Unknown key names are kept as-is and simply never match input; they do not panic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyName(pub String);

impl KeyName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// Primary and optional alias keys for every action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBindings {
    pub accelerate: Vec<KeyName>,
    pub brake: Vec<KeyName>,
    pub steer_left: Vec<KeyName>,
    pub steer_right: Vec<KeyName>,
    pub pipe_left: Vec<KeyName>,
    pub pipe_right: Vec<KeyName>,
    pub laser: Vec<KeyName>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            accelerate: vec![KeyName::new("KeyW"), KeyName::new("Up")],
            brake: vec![KeyName::new("KeyS"), KeyName::new("Down")],
            steer_left: vec![KeyName::new("KeyA"), KeyName::new("Left")],
            steer_right: vec![KeyName::new("KeyD"), KeyName::new("Right")],
            pipe_left: vec![KeyName::new("KeyQ")],
            pipe_right: vec![KeyName::new("KeyE")],
            laser: vec![KeyName::new("Space")],
        }
    }
}

impl KeyBindings {
    /// Keys bound to an action (primary + aliases).
    pub fn keys_for(&self, action: Action) -> &[KeyName] {
        match action {
            Action::Accelerate => &self.accelerate,
            Action::Brake => &self.brake,
            Action::SteerLeft => &self.steer_left,
            Action::SteerRight => &self.steer_right,
            Action::PipeLeft => &self.pipe_left,
            Action::PipeRight => &self.pipe_right,
            Action::Laser => &self.laser,
        }
    }

    /// Replaces the primary binding for an action (keeps aliases after the first).
    pub fn set_primary(&mut self, action: Action, key: KeyName) {
        let slot = match action {
            Action::Accelerate => &mut self.accelerate,
            Action::Brake => &mut self.brake,
            Action::SteerLeft => &mut self.steer_left,
            Action::SteerRight => &mut self.steer_right,
            Action::PipeLeft => &mut self.pipe_left,
            Action::PipeRight => &mut self.pipe_right,
            Action::Laser => &mut self.laser,
        };
        if slot.is_empty() {
            slot.push(key);
        } else {
            slot[0] = key;
        }
    }

    /// Returns true if `key` is bound to `action`.
    pub fn matches(&self, action: Action, key: &str) -> bool {
        self.keys_for(action).iter().any(|k| k.0 == key)
    }

    /// Serializes bindings to pretty JSON.
    pub fn to_json_string(&self) -> Result<String, BindingsError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses bindings from JSON. Unknown fields are ignored by serde defaults
    /// when using this struct; missing fields fail with a parse error, so prefer
    /// [`KeyBindings::from_json_lenient`].
    pub fn from_json_str(json: &str) -> Result<Self, BindingsError> {
        Ok(serde_json::from_str(json)?)
    }

    /// Parses bindings, filling any missing action lists from defaults.
    /// Unknown key *names* inside arrays are preserved and never panic.
    pub fn from_json_lenient(json: &str) -> Result<Self, BindingsError> {
        #[derive(Deserialize)]
        struct Partial {
            accelerate: Option<Vec<KeyName>>,
            brake: Option<Vec<KeyName>>,
            steer_left: Option<Vec<KeyName>>,
            steer_right: Option<Vec<KeyName>>,
            pipe_left: Option<Vec<KeyName>>,
            pipe_right: Option<Vec<KeyName>>,
            laser: Option<Vec<KeyName>>,
        }
        let partial: Partial = serde_json::from_str(json)?;
        let defaults = Self::default();
        Ok(Self {
            accelerate: partial.accelerate.unwrap_or(defaults.accelerate),
            brake: partial.brake.unwrap_or(defaults.brake),
            steer_left: partial.steer_left.unwrap_or(defaults.steer_left),
            steer_right: partial.steer_right.unwrap_or(defaults.steer_right),
            pipe_left: partial.pipe_left.unwrap_or(defaults.pipe_left),
            pipe_right: partial.pipe_right.unwrap_or(defaults.pipe_right),
            laser: partial.laser.unwrap_or(defaults.laser),
        })
    }

    /// Loads from `path`, or writes defaults there if the file is missing.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, BindingsError> {
        let path = path.as_ref();
        if path.exists() {
            let text = fs::read_to_string(path)?;
            return Self::from_json_lenient(&text);
        }
        let bindings = Self::default();
        bindings.save(path)?;
        Ok(bindings)
    }

    /// Writes bindings as pretty JSON, creating parent directories as needed.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), BindingsError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_json_string()?)?;
        Ok(())
    }

    /// Resolves the default config path relative to the process working directory.
    pub fn default_path() -> PathBuf {
        PathBuf::from(DEFAULT_BINDINGS_PATH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn serialize_and_load_roundtrip() {
        let original = KeyBindings::default();
        let json = original.to_json_string().expect("serialize");
        let loaded = KeyBindings::from_json_str(&json).expect("parse");
        assert_eq!(original, loaded);
    }

    #[test]
    fn unknown_keys_do_not_panic() {
        let json = r#"{
            "accelerate": ["KeyW", "NotARealKey"],
            "brake": ["TotallyFake"],
            "steer_left": ["KeyA"],
            "steer_right": ["KeyD"],
            "pipe_left": ["KeyQ"],
            "pipe_right": ["KeyE"],
            "laser": ["Space", "MysteryButton"]
        }"#;
        let bindings = KeyBindings::from_json_lenient(json).expect("parse");
        assert!(bindings.matches(Action::Accelerate, "KeyW"));
        assert!(bindings.matches(Action::Accelerate, "NotARealKey"));
        assert!(!bindings.matches(Action::Accelerate, "KeyS"));
        // Matching an unknown name is fine; it just never fires from real input.
        assert!(bindings.matches(Action::Brake, "TotallyFake"));
    }

    #[test]
    fn load_or_create_writes_missing_file() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("bike-game-bindings-{nanos}"));
        let path = dir.join("keys.json");
        let bindings = KeyBindings::load_or_create(&path).expect("create");
        assert!(path.exists());
        assert_eq!(bindings, KeyBindings::default());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let json = r#"{ "accelerate": ["KeyZ"] }"#;
        let bindings = KeyBindings::from_json_lenient(json).expect("parse");
        assert_eq!(bindings.accelerate[0].0, "KeyZ");
        assert_eq!(bindings.laser[0].0, "Space");
    }
}
