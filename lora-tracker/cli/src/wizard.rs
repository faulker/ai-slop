//! Interactive `setup`: asks what hardware/accounts you have and writes a
//! valid `device.toml`. Prompting ([`ask`]) is kept apart from building the
//! config ([`build`]) so the result is testable without a terminal.

use anyhow::Result;
use dialoguer::{Confirm, Input, Password, Select};

use crate::device_toml::{DeviceToml, Lora, Lte, Meshtastic, Role, Server, Wifi, WifiNetwork};
use crate::pair::parse_key;

/// Everything the wizard collects. `None`/empty means "skip that path".
#[derive(Debug, Clone, Default)]
pub struct Answers {
    pub role: Option<Role>,
    pub lora: bool,
    pub wifi: Vec<(String, String)>,
    pub allow_open_wifi: bool,
    pub server_url: Option<String>,
    pub meshtastic: bool,
    pub lte_apn: Option<String>,
    /// User-supplied key; `None` means generate one.
    pub own_key: Option<[u8; 32]>,
}

/// LoRa defaults for US 915 MHz under Part 15.
fn default_lora() -> Lora {
    Lora { frequency_hz: 915_000_000, tx_power_dbm: 14, interval_s: 30, stationary_interval_s: 600 }
}

/// Turns answers into a config. `device_id`, `random_key` and `mesh_psk` are
/// passed in so tests are deterministic.
pub fn build(a: &Answers, device_id: u32, random_key: [u8; 32], mesh_psk: [u8; 16]) -> DeviceToml {
    let role = a.role.unwrap_or(Role::Tracker);
    let tracker = role == Role::Tracker;
    let wifi = (!a.wifi.is_empty() || (tracker && a.allow_open_wifi)).then(|| Wifi {
        known: a
            .wifi
            .iter()
            .map(|(ssid, password)| WifiNetwork { ssid: ssid.clone(), password: password.clone() })
            .collect(),
        allow_open: tracker && a.allow_open_wifi,
    });
    DeviceToml {
        device_id,
        role,
        // Gateways get their keys through `pair`, one per tracker.
        device_key: tracker.then(|| hex::encode(a.own_key.unwrap_or(random_key))),
        lora: a.lora.then(default_lora),
        wifi,
        server: a.server_url.clone().map(|url| Server { url }),
        meshtastic: (tracker && a.meshtastic).then(|| Meshtastic {
            channel_name: "tracker".into(),
            psk: hex::encode(mesh_psk),
            interval_s: 900,
        }),
        findmy: None,
        lte: a.lte_apn.clone().filter(|_| tracker).map(|apn| Lte { apn, interval_s: 300 }),
        peers: Vec::new(),
    }
}

/// Serializes a wizard result with a short header explaining how to change it.
pub fn render(d: &DeviceToml) -> Result<String> {
    Ok(format!(
        "# lora-tracker device config. Change it with `lora-tracker edit <this file>`.\n\
         # Any section can be removed; only that path turns off.\n\
         # Contains secrets (keys, WiFi passwords): don't share or commit it.\n\n{}",
        toml::to_string_pretty(d)?
    ))
}

/// Optional text input: empty answer means `None`.
fn optional(prompt: &str) -> Result<Option<String>> {
    let s: String = Input::new().with_prompt(prompt).allow_empty(true).interact_text()?;
    Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty()))
}

/// Runs the prompts.
pub fn ask() -> Result<Answers> {
    let mut a = Answers::default();
    let role = Select::new()
        .with_prompt("What is this board?")
        .items(&["Tracker (rides on the vehicle)", "Gateway (stays at home, forwards to the server)"])
        .default(0)
        .interact()?;
    a.role = Some(if role == 0 { Role::Tracker } else { Role::Gateway });
    let tracker = role == 0;

    a.lora = Confirm::new().with_prompt("Use the LoRa link between tracker and gateway?").default(true).interact()?;

    println!("WiFi networks to use, in order (press Enter on an empty name to finish).");
    while a.wifi.len() < tracker_proto::config::MAX_WIFI {
        let Some(ssid) = optional("  Network name")? else { break };
        let pw = Password::new()
            .with_prompt("  Password (empty for an open network)")
            .allow_empty_password(true)
            .interact()?;
        a.wifi.push((ssid, pw));
    }
    if tracker {
        a.allow_open_wifi = Confirm::new()
            .with_prompt("Also try open WiFi networks you haven't listed? (often blocked by login pages)")
            .default(false)
            .interact()?;
    }
    a.server_url = optional("Server URL, e.g. http://192.168.1.10:8080 (Enter to skip)")?;

    if tracker {
        a.meshtastic = Confirm::new()
            .with_prompt("Send Meshtastic fallback packets so other mesh nodes can relay?")
            .default(false)
            .interact()?;
        a.lte_apn = optional("LTE APN for the SIM7080G, e.g. hologram (Enter if no LTE module)")?;

        let key_choice = Select::new()
            .with_prompt("Encryption key for this tracker")
            .items(&["Generate a random key (recommended)", "Enter my own key (64 hex characters)"])
            .default(0)
            .interact()?;
        if key_choice == 1 {
            let k: String = Input::new()
                .with_prompt("  Key")
                .validate_with(|s: &String| parse_key(s).map(|_| ()).map_err(|e| e.to_string()))
                .interact_text()?;
            a.own_key = Some(parse_key(&k)?);
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracker_proto::config::{PathPlan, Disabled};

    const KEY: [u8; 32] = [0x22; 32];

    /// Round-trips through render + parse + the real path rules.
    fn paths_of(d: &DeviceToml) -> tracker_proto::config::Paths {
        let text = render(d).unwrap();
        let parsed: DeviceToml = toml::from_str(&text).unwrap();
        parsed.to_config().unwrap().paths()
    }

    #[test]
    fn tracker_with_everything() {
        let a = Answers {
            role: Some(Role::Tracker),
            lora: true,
            wifi: vec![("home".into(), "pw".into())],
            server_url: Some("http://srv:8080".into()),
            meshtastic: true,
            lte_apn: Some("hologram".into()),
            ..Default::default()
        };
        let d = build(&a, 1, KEY, [3; 16]);
        assert_eq!(d.device_key.as_deref(), Some("22".repeat(32).as_str()));
        let p = paths_of(&d);
        assert_eq!((p.lora, p.wifi, p.lte, p.meshtastic), (PathPlan::Enabled, PathPlan::Enabled, PathPlan::Enabled, PathPlan::Enabled));
    }

    #[test]
    fn skipped_answers_disable_only_those_paths() {
        let a = Answers { role: Some(Role::Tracker), lora: true, ..Default::default() };
        let p = paths_of(&build(&a, 1, KEY, [0; 16]));
        assert_eq!(p.lora, PathPlan::Enabled);
        assert_eq!(p.wifi, PathPlan::Disabled(Disabled::NotConfigured));
        assert_eq!(p.lte, PathPlan::Disabled(Disabled::NotConfigured));
    }

    #[test]
    fn own_key_is_used() {
        let a = Answers { role: Some(Role::Tracker), own_key: Some([0x77; 32]), ..Default::default() };
        assert_eq!(build(&a, 1, KEY, [0; 16]).device_key, Some("77".repeat(32)));
    }

    #[test]
    fn gateway_ignores_tracker_only_answers() {
        let a = Answers {
            role: Some(Role::Gateway),
            lora: true,
            wifi: vec![("home".into(), "pw".into())],
            allow_open_wifi: true,
            meshtastic: true,
            lte_apn: Some("x".into()),
            server_url: Some("http://srv".into()),
            ..Default::default()
        };
        let d = build(&a, 2, KEY, [0; 16]);
        assert!(d.device_key.is_none() && d.meshtastic.is_none() && d.lte.is_none());
        assert!(!d.wifi.as_ref().unwrap().allow_open);
        let p = paths_of(&d);
        assert_eq!((p.lora, p.wifi), (PathPlan::Enabled, PathPlan::Enabled));
    }
}
