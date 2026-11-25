use std::collections::BTreeMap;

use crate::bmw::modules;
use crate::bmw::settings::{Setting, SettingsFile};
use crate::error::Result;
use crate::protocol::uds;
use crate::transport::elm327::Elm327;

/// Read every DID that a mapped setting points at, once per (module, DID).
/// Read-only: default session, ReadDataByIdentifier only.
pub async fn read_mapped(
    elm: &mut Elm327,
    file: &SettingsFile,
) -> Result<BTreeMap<(String, u16), Vec<u8>>> {
    let mut values = BTreeMap::new();
    for s in file.settings.iter().filter(|s| s.is_mapped()) {
        let (Some(module), Some(did)) = (&s.module, s.did) else { continue };
        let key = (module.to_uppercase(), did);
        if values.contains_key(&key) {
            continue;
        }
        let address = u8::from_str_radix(module, 16).unwrap_or(0);
        elm.set_header(&modules::target(address)).await?;
        if let Ok(resp) = uds::send_uds(elm, &uds::read_data_by_identifier(did)).await {
            if resp.len() >= 3 {
                values.insert(key, resp[3..].to_vec());
            }
        }
    }
    elm.set_header("7DF").await?;
    Ok(values)
}

/// Print settings in plain language, optionally with current values from the bike.
/// `module` limits output to one module address.
pub fn print(
    file: &SettingsFile,
    module: Option<&str>,
    current: &BTreeMap<(String, u16), Vec<u8>>,
) {
    let shown: Vec<&Setting> = file
        .settings
        .iter()
        .filter(|s| match module {
            Some(m) => s.module.as_deref().is_some_and(|sm| sm.eq_ignore_ascii_case(m)),
            None => true,
        })
        .collect();

    println!("{} settings ({} shown)\n", file.model, shown.len());
    for s in shown {
        let status = if s.verified { "verified" } else { "unverified" };
        println!("{}  [{}, {}]", s.name, s.risk.label(), status);
        println!("  What it is:   {}", s.description);
        println!("  What changes: {}", s.effect);
        match (&s.module, s.did) {
            (Some(m), Some(did)) => {
                let name = file.module_name(m).unwrap_or("unknown module");
                println!("  Where:        module 0x{} ({}), DID 0x{:04X}", m.to_uppercase(), name, did);
                if let Some(data) = current.get(&(m.to_uppercase(), did)) {
                    let value = s
                        .extract(data)
                        .map(|v| s.describe_value(v))
                        .unwrap_or_else(|| uds::hex_string(data));
                    println!("  Current:      {}", value);
                }
            }
            _ => println!("  Where:        not mapped on this bike yet"),
        }
        println!("  Source:       {}\n", s.source);
    }
}
