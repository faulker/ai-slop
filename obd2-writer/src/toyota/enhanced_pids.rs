use std::path::Path;
use std::sync::OnceLock;

use serde::Deserialize;
use tracing::warn;

use crate::error::{Error, Result};
use crate::protocol::uds;
use crate::transport::elm327::Elm327;

static DID_CACHE: OnceLock<Vec<DidDefinition>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
pub struct DidDefinition {
    pub id: u16,
    pub name: String,
    pub unit: String,
    pub formula: String,
    #[serde(default = "default_ecu")]
    #[allow(dead_code)]
    pub ecu: String,
    #[serde(default)]
    pub writable: bool,
    pub data_length: Option<usize>,
    pub min_value: Option<u64>,
    pub max_value: Option<u64>,
}

fn default_ecu() -> String {
    "7E0".to_string()
}

#[derive(Debug, Deserialize)]
pub struct DidConfig {
    pub did: Vec<DidDefinition>,
}

/// Load Toyota DID definitions from a TOML file.
pub fn load_dids(path: &Path) -> Result<Vec<DidDefinition>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| Error::Config(e.to_string()))?;
    let config: DidConfig = toml::from_str(&content).map_err(|e| Error::Config(e.to_string()))?;
    Ok(config.did)
}

/// Evaluate a simple formula like "A - 40", "(A * 256 + B) / 4", etc.
/// Returns `None` if the formula is not recognized.
fn evaluate_formula(formula: &str, data: &[u8]) -> Option<f64> {
    let a = data.first().copied().unwrap_or(0) as f64;
    let b = data.get(1).copied().unwrap_or(0) as f64;

    // Normalize whitespace so "A  -  40" matches "A - 40"
    let normalized: String = formula.split_whitespace().collect::<Vec<_>>().join(" ");
    let f = normalized.as_str();
    let result = match f {
        "A" => a,
        "B" => b,
        "A - 40" => a - 40.0,
        "B - 40" => b - 40.0,
        "A * 100 / 255" | "A * 100.0 / 255.0" => a * 100.0 / 255.0,
        "(A * 256 + B) / 4" => (a * 256.0 + b) / 4.0,
        "(A * 256 + B) / 100" => (a * 256.0 + b) / 100.0,
        "(A * 256 + B) / 1000" => (a * 256.0 + b) / 1000.0,
        "(A * 256 + B)" => a * 256.0 + b,
        "A / 2 - 64" => a / 2.0 - 64.0,
        "A * 3 / 255" => a * 3.0 / 255.0,
        "(A - 128) * 100 / 128" => (a - 128.0) * 100.0 / 128.0,
        _ => return None,
    };
    Some(result)
}

pub fn cached_dids() -> &'static [DidDefinition] {
    DID_CACHE.get_or_init(|| {
        let dids_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("toyota_dids.toml")))
            .filter(|p| p.exists())
            .unwrap_or_else(|| Path::new("toyota_dids.toml").to_path_buf());
        load_dids(&dids_path).unwrap_or_default()
    })
}

/// Read a Toyota enhanced DID (Mode 22) and print the result.
pub async fn read_enhanced_did(elm: &mut Elm327, did_hex: &str, ecu: &str) -> Result<()> {
    let did = u16::from_str_radix(did_hex.trim_start_matches("0x"), 16)
        .map_err(|_| Error::Config(format!("invalid DID hex: {}", did_hex)))?;

    // Set target ECU
    elm.set_header(ecu).await?;

    let request = uds::read_data_by_identifier(did);
    let response = uds::send_uds(elm, &request).await?;

    // Response: [0x62, DID_HI, DID_LO, data...]
    if response.len() < 3 {
        return Err(Error::Protocol("response too short".into()));
    }

    let data = &response[3..];

    let dids = cached_dids();

    if let Some(def) = dids.iter().find(|d| d.id == did) {
        match evaluate_formula(&def.formula, data) {
            Some(value) => println!("{}: {:.1} {}", def.name, value, def.unit),
            None => {
                warn!(formula = def.formula.as_str(), "unrecognized formula, showing raw data");
                println!("{}: {} (raw, formula '{}' not recognized)", def.name, uds::hex_string(data), def.formula);
            }
        }
    } else {
        println!("DID 0x{:04X}: {}", did, uds::hex_string(data));
    }

    // Reset header to default OBD broadcast to avoid affecting subsequent commands
    elm.set_header("7DF").await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_formula_simple() {
        assert_eq!(evaluate_formula("A - 40", &[80]), Some(40.0));
    }

    #[test]
    fn test_evaluate_formula_two_byte() {
        assert_eq!(evaluate_formula("(A * 256 + B) / 4", &[0x1A, 0xF8]), Some(1726.0));
    }

    #[test]
    fn test_evaluate_formula_percentage() {
        let result = evaluate_formula("A * 100 / 255", &[127]).unwrap();
        assert!((result - 49.8).abs() < 0.1);
    }

    #[test]
    fn test_evaluate_formula_extra_whitespace() {
        // Verify whitespace normalization works
        assert_eq!(evaluate_formula("A  -  40", &[80]), Some(40.0));
        assert_eq!(evaluate_formula("  A - 40  ", &[80]), Some(40.0));
        assert_eq!(evaluate_formula("(A * 256 + B)  /  4", &[0x1A, 0xF8]), Some(1726.0));
    }

    #[test]
    fn test_evaluate_formula_unknown() {
        assert_eq!(evaluate_formula("BOGUS FORMULA", &[42]), None);
    }
}
