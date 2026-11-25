use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// How much harm a wrong value could do. Only `Cosmetic` and `Comfort`
/// settings are ever writable; the rest are shown for information only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Display-only: screens, tiles, logos.
    Cosmetic,
    /// Convenience behavior: lights, pop-ups, reminders.
    Comfort,
    /// Affects ABS, DSC, ESA, engine or anything that changes how the bike rides.
    Safety,
    /// Never touch (immobilizer, odometer, emissions, VIN).
    Never,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Cosmetic => "cosmetic",
            Risk::Comfort => "comfort",
            Risk::Safety => "SAFETY (read-only)",
            Risk::Never => "NEVER (blocked)",
        }
    }
}

/// A diagnostic module found on the bike.
#[derive(Debug, Clone, Deserialize)]
pub struct Module {
    /// Extended address as hex, e.g. "60".
    pub address: String,
    pub name: String,
}

/// One codable setting. Community entries start with `module`/`did` unset
/// and `verified = false`; they become writable only after the snapshot/diff
/// workflow confirms exactly which bytes they control on this bike.
#[derive(Debug, Clone, Deserialize)]
pub struct Setting {
    pub name: String,
    /// Plain-English summary of what the setting is.
    pub description: String,
    /// What you will actually notice on the bike when it changes.
    pub effect: String,
    pub risk: Risk,
    /// Where the claim comes from (URL, or "diff YYYY-MM-DD" for our own findings).
    pub source: String,
    #[serde(default)]
    pub verified: bool,
    pub module: Option<String>,
    pub did: Option<u16>,
    /// Byte offset within the DID data.
    pub byte: Option<usize>,
    /// Bits within that byte that belong to this setting.
    pub mask: Option<u8>,
    /// Masked value (hex, e.g. "01") -> meaning.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

impl Setting {
    /// True once the setting is tied to a concrete module and DID.
    pub fn is_mapped(&self) -> bool {
        self.module.is_some() && self.did.is_some()
    }

    /// Masked value of this setting inside raw DID data, if mapped to a byte.
    pub fn extract(&self, data: &[u8]) -> Option<u8> {
        let byte = *data.get(self.byte?)?;
        Some(byte & self.mask.unwrap_or(0xFF))
    }

    /// Human meaning of a masked value, falling back to hex.
    pub fn describe_value(&self, value: u8) -> String {
        let key = format!("{:02X}", value);
        match self.values.get(&key) {
            Some(meaning) => format!("{} ({})", meaning, key),
            None => key,
        }
    }
}

/// Contents of a vehicle settings file such as `bmw_k51.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsFile {
    pub model: String,
    #[serde(default, rename = "module")]
    pub modules: Vec<Module>,
    #[serde(default, rename = "setting")]
    pub settings: Vec<Setting>,
}

impl SettingsFile {
    pub fn parse(content: &str) -> Result<Self> {
        toml::from_str(content).map_err(|e| Error::Config(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("failed to read {}: {}", path.display(), e)))?;
        Self::parse(&content)
    }

    /// Name of a module by its hex address, if known.
    pub fn module_name(&self, address: &str) -> Option<&str> {
        self.modules
            .iter()
            .find(|m| m.address.eq_ignore_ascii_case(address))
            .map(|m| m.name.as_str())
    }
}

/// Locate `bmw_k51.toml` next to the executable, falling back to the working directory.
pub fn default_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("bmw_k51.toml")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("bmw_k51.toml"))
}

/// Refuse a write unless the setting is verified, mapped, and low risk.
/// This is the single gate every BMW write must pass.
#[allow(dead_code)] // used by the Phase 3 write path once settings are verified
pub fn check_writable(setting: &Setting) -> Result<()> {
    let refuse = |why: &str| Err(Error::Config(format!("'{}' is not writable: {}", setting.name, why)));
    match setting.risk {
        Risk::Never => return refuse("marked never-touch"),
        Risk::Safety => return refuse("safety-related settings are read-only in this tool"),
        Risk::Cosmetic | Risk::Comfort => {}
    }
    if !setting.verified {
        return refuse("not yet verified on this bike with a snapshot diff");
    }
    if !setting.is_mapped() || setting.byte.is_none() {
        return refuse("module, DID and byte are not all known");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
model = "K51"

[[module]]
address = "60"
name = "Instrument cluster"

[[setting]]
name = "Mapped cosmetic"
description = "d"
effect = "e"
risk = "cosmetic"
source = "test"
verified = true
module = "60"
did = 0x3000
byte = 1
mask = 0x0C
values = { "04" = "On", "00" = "Off" }

[[setting]]
name = "Unmapped community"
description = "d"
effect = "e"
risk = "comfort"
source = "https://example.com"
"#;

    fn sample() -> SettingsFile {
        SettingsFile::parse(SAMPLE).unwrap()
    }

    fn with(mut s: Setting, f: impl FnOnce(&mut Setting)) -> Setting {
        f(&mut s);
        s
    }

    #[test]
    fn test_parse_sample() {
        let file = sample();
        assert_eq!(file.model, "K51");
        assert_eq!(file.module_name("60"), Some("Instrument cluster"));
        assert_eq!(file.settings.len(), 2);
        assert!(!file.settings[1].verified);
        assert!(!file.settings[1].is_mapped());
    }

    #[test]
    fn test_extract_and_describe() {
        let s = &sample().settings[0];
        assert_eq!(s.extract(&[0xFF, 0x07]), Some(0x04));
        assert_eq!(s.describe_value(0x04), "On (04)");
        assert_eq!(s.describe_value(0x08), "08");
        assert_eq!(s.extract(&[0xFF]), None);
    }

    #[test]
    fn test_gate_allows_verified_mapped_cosmetic() {
        assert!(check_writable(&sample().settings[0]).is_ok());
    }

    #[test]
    fn test_gate_blocks_safety_and_never() {
        let base = sample().settings[0].clone();
        assert!(check_writable(&with(base.clone(), |s| s.risk = Risk::Safety)).is_err());
        assert!(check_writable(&with(base, |s| s.risk = Risk::Never)).is_err());
    }

    #[test]
    fn test_gate_blocks_unverified_or_unmapped() {
        let base = sample().settings[0].clone();
        assert!(check_writable(&with(base.clone(), |s| s.verified = false)).is_err());
        assert!(check_writable(&with(base.clone(), |s| s.did = None)).is_err());
        assert!(check_writable(&with(base, |s| s.byte = None)).is_err());
        assert!(check_writable(&sample().settings[1]).is_err());
    }

    #[test]
    fn test_shipped_k51_file_parses_and_nothing_is_writable_yet() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("bmw_k51.toml");
        let file = SettingsFile::load(&path).unwrap();
        assert_eq!(file.model, "K51");
        assert!(!file.settings.is_empty());
        for s in &file.settings {
            assert!(!s.source.is_empty(), "{} has no source", s.name);
            if !s.verified {
                assert!(check_writable(s).is_err());
            }
        }
    }
}
