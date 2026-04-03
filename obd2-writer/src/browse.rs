use dialoguer::FuzzySelect;

use crate::obd::pid;
use crate::toyota::enhanced_pids;

/// Interactive PID browser. Returns the selected PID name, or None if cancelled.
pub fn browse_pids() -> Option<String> {
    let items: Vec<String> = pid::PIDS
        .iter()
        .map(|p| format!("{:16} {:6} (0x{:02X})", p.name, p.unit, p.pid))
        .collect();

    if items.is_empty() {
        println!("No PIDs defined.");
        return None;
    }

    let selection = FuzzySelect::new()
        .with_prompt("Select a PID")
        .items(&items)
        .default(0)
        .interact_opt()
        .ok()
        .flatten()?;

    Some(pid::PIDS[selection].name.to_string())
}

/// Interactive DID browser. Returns (did_hex, ecu) or None if cancelled.
pub fn browse_dids() -> Option<(String, String)> {
    let dids = enhanced_pids::cached_dids();

    if dids.is_empty() {
        println!("No DIDs defined in toyota_dids.toml.");
        return None;
    }

    let items: Vec<String> = dids
        .iter()
        .map(|d| {
            let writable = if d.writable { " [writable]" } else { "" };
            let cat = d.category.as_deref().unwrap_or("");
            let cat_prefix = if cat.is_empty() {
                String::new()
            } else {
                format!("[{}] ", cat)
            };
            let desc = d
                .description
                .as_deref()
                .map(|s| format!(" — {}", s))
                .unwrap_or_default();
            format!(
                "{}{} — {} (0x{:04X}, ECU: {}){}{}",
                cat_prefix, d.name, d.unit, d.id, d.ecu, writable, desc
            )
        })
        .collect();

    let selection = FuzzySelect::new()
        .with_prompt("Select a DID")
        .items(&items)
        .default(0)
        .interact_opt()
        .ok()
        .flatten()?;

    let did = &dids[selection];
    Some((format!("{:04X}", did.id), did.ecu.clone()))
}
