//! `lora-tracker`: set up, pair, edit, and flash tracker and gateway boards.
//!
//! Typical flow:
//! - `lora-tracker setup -o tracker.toml` and `lora-tracker setup -o gateway.toml`
//! - `lora-tracker pair tracker.toml gateway.toml` (shares one key)
//! - `lora-tracker flash tracker.toml` (firmware + config) or `flash-config` (config only)
//! - later: `lora-tracker edit tracker.toml` to change settings and re-flash

mod device_toml;
mod flash;
mod pair;
mod secret;
mod wizard;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::Confirm;
use rand::RngCore;
use tracker_proto::config::{self as pc, PathPlan};

use device_toml::{render_template, DeviceToml, Role};
use pair::KeyChoice;
use secret::write_secret;

#[derive(Parser)]
#[command(name = "lora-tracker", about = "Set up, pair, edit, and flash lora-tracker boards")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum RoleArg {
    Tracker,
    Gateway,
}

#[derive(Subcommand)]
enum Cmd {
    /// Guided setup: answer a few questions, get a working config, optionally flash it.
    Setup {
        #[arg(short, long, default_value = "device.toml")]
        out: PathBuf,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
        /// Serial port, if espflash can't pick it automatically.
        #[arg(long)]
        port: Option<String>,
    },
    /// Share one encryption key between a tracker and a gateway.
    Pair {
        tracker: PathBuf,
        gateway: PathBuf,
        /// Use this key (64 hex characters) instead of the tracker's current one.
        #[arg(long, conflicts_with = "new_key")]
        key: Option<String>,
        /// Generate a fresh random key, replacing the current one on both.
        #[arg(long)]
        new_key: bool,
    },
    /// Open a config in $EDITOR, validate it, and offer to re-flash.
    /// If the file doesn't exist, a basic profile with fresh secrets is created first.
    Edit {
        file: PathBuf,
        /// Role for a newly created profile; asked interactively if omitted.
        #[arg(long, value_enum)]
        role: Option<RoleArg>,
        #[arg(long)]
        port: Option<String>,
    },
    /// Print which paths a config enables, and why others are off.
    Show { file: PathBuf },
    /// Flash firmware and config to a connected board.
    Flash {
        file: PathBuf,
        /// Firmware ELF; defaults to this checkout's release build for the detected board (V2 or V3).
        #[arg(long)]
        firmware: Option<PathBuf>,
        #[arg(long)]
        port: Option<String>,
    },
    /// Flash only the config (seconds, no rebuild; firmware is untouched).
    FlashConfig {
        file: PathBuf,
        #[arg(long)]
        port: Option<String>,
    },
    /// Non-interactive: write a template config with fresh secrets.
    Init {
        #[arg(long, value_enum)]
        role: RoleArg,
        /// Use this device id instead of a random one.
        #[arg(long)]
        device_id: Option<u32>,
        #[arg(short, long, default_value = "device.toml")]
        out: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Pack a config into a flashable blob file without flashing.
    Pack {
        file: PathBuf,
        #[arg(short, long, default_value = "device.bin")]
        out: PathBuf,
    },
}

/// Reads and validates a device.toml.
fn load(path: &Path) -> Result<pc::Config> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed: DeviceToml = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    parsed.to_config().with_context(|| format!("in {}", path.display()))
}

/// Prints the path summary for a config.
fn print_paths(cfg: &pc::Config) {
    let p = cfg.paths();
    println!("device_id {} ({:?})", cfg.device_id, cfg.role);
    for (name, plan) in [
        ("lora", p.lora),
        ("wifi", p.wifi),
        ("lte", p.lte),
        ("meshtastic", p.meshtastic),
        ("findmy", p.findmy),
        ("ble_status", p.ble_status),
    ] {
        let state = match plan {
            PathPlan::Enabled => "on".to_string(),
            PathPlan::Disabled(why) => format!("off ({why:?})"),
        };
        println!("  {name:<11} {state}");
    }
    if cfg.role == pc::Role::Gateway {
        if cfg.peers.is_empty() {
            println!("  paired trackers: none yet (run `lora-tracker pair <tracker.toml> <gateway.toml>`)");
        } else {
            let ids: Vec<String> = cfg.peers.iter().map(|p| p.device_id.to_string()).collect();
            println!("  paired trackers: {}", ids.join(", "));
        }
    }
}

/// Refuses to clobber a secrets file unless `force` is set.
fn check_overwrite(out: &Path, force: bool) -> Result<()> {
    if out.exists() && !force {
        bail!("{} exists; pass --force to overwrite (it holds secrets)", out.display());
    }
    Ok(())
}

/// Random 32-byte key from the OS RNG.
fn random_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut k);
    k
}

impl From<RoleArg> for Role {
    /// Maps the CLI flag to the config role.
    fn from(r: RoleArg) -> Role {
        match r {
            RoleArg::Tracker => Role::Tracker,
            RoleArg::Gateway => Role::Gateway,
        }
    }
}

/// Writes a basic profile (template with fresh secrets, optional sections
/// commented out) to `path`, returning its device id.
fn create_profile(path: &Path, role: Role, device_id: Option<u32>) -> Result<u32> {
    let id = device_id.unwrap_or_else(|| rand::rngs::OsRng.next_u32());
    let key = (role == Role::Tracker).then(random_key);
    let mut psk = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut psk);
    write_secret(path, render_template(id, role, key, psk).as_bytes())?;
    Ok(id)
}

/// Role suggested for a new file: gateway if its name says so, else tracker.
fn suggested_role(path: &Path) -> Role {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.contains("gateway") {
        Role::Gateway
    } else {
        Role::Tracker
    }
}

/// Asks which role a new profile should have, preselecting [`suggested_role`].
fn ask_role(path: &Path) -> Result<Role> {
    if !std::io::stdin().is_terminal() {
        bail!("{} doesn't exist; pass --role tracker or --role gateway to create it", path.display());
    }
    let default = usize::from(suggested_role(path) == Role::Gateway);
    let pick = dialoguer::Select::new()
        .with_prompt(format!("{} doesn't exist yet. What is this board?", path.display()))
        .items(&["Tracker", "Gateway"])
        .default(default)
        .interact()?;
    Ok(if pick == 0 { Role::Tracker } else { Role::Gateway })
}

/// Pairs two config files in place and prints the matching server entry.
fn pair_files(tracker: &Path, gateway: &Path, choice: KeyChoice) -> Result<()> {
    let t = std::fs::read_to_string(tracker).with_context(|| format!("reading {}", tracker.display()))?;
    let g = std::fs::read_to_string(gateway).with_context(|| format!("reading {}", gateway.display()))?;
    let p = pair::pair(&t, &g, choice, random_key())?;
    write_secret(tracker, p.tracker.as_bytes())?;
    write_secret(gateway, p.gateway.as_bytes())?;
    println!("paired tracker {} with {}", p.tracker_id, gateway.display());
    println!("re-flash both configs: `lora-tracker flash-config <file>`");
    println!("server entry (add to server.toml, then restart lora-tracker-server):");
    println!("  [[device]]\n  device_id = {}\n  key = \"{}\"", p.tracker_id, hex::encode(p.key));
    Ok(())
}

/// Asks whether to flash a config now, and does it.
fn offer_flash(cfg: &pc::Config, port: Option<&str>) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        println!("not flashing (no terminal to confirm); run `lora-tracker flash-config <file>` when ready");
        return Ok(());
    }
    if Confirm::new().with_prompt("Flash this config to a connected board now?").default(false).interact()? {
        flash::flash_config(cfg, port)?;
        println!("config flashed");
    }
    Ok(())
}

/// Opens `file` in the user's editor until it validates or they give up.
fn edit(file: &Path, port: Option<&str>) -> Result<()> {
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
    loop {
        // $EDITOR may carry flags, e.g. "code --wait".
        let mut parts = editor.split_whitespace();
        let program = parts.next().context("$EDITOR is empty")?;
        let status = std::process::Command::new(program)
            .args(parts)
            .arg(file)
            .status()
            .with_context(|| format!("launching editor `{editor}`"))?;
        if !status.success() {
            bail!("editor exited with {status}; file left as is");
        }
        match load(file) {
            Ok(cfg) => {
                print_paths(&cfg);
                return offer_flash(&cfg, port);
            }
            Err(e) => {
                eprintln!("config has a problem: {e:#}");
                if !Confirm::new().with_prompt("Open the editor again to fix it?").default(true).interact()? {
                    bail!("{} is not valid; nothing was flashed", file.display());
                }
            }
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Setup { out, force, port } => {
            check_overwrite(&out, force)?;
            let answers = wizard::ask()?;
            let mut psk = [0u8; 16];
            rand::rngs::OsRng.fill_bytes(&mut psk);
            let device = wizard::build(&answers, rand::rngs::OsRng.next_u32(), random_key(), psk);
            write_secret(&out, wizard::render(&device)?.as_bytes())?;
            println!("\nwrote {}", out.display());

            if device.role == Role::Gateway {
                let tracker: String = dialoguer::Input::new()
                    .with_prompt("Pair with a tracker now? Path to its .toml (Enter to skip)")
                    .allow_empty(true)
                    .interact_text()?;
                if !tracker.trim().is_empty() {
                    pair_files(Path::new(tracker.trim()), &out, KeyChoice::Existing)?;
                }
            }
            let cfg = load(&out)?;
            print_paths(&cfg);
            offer_flash(&cfg, port.as_deref())?;
            println!("change settings later with `lora-tracker edit {}`", out.display());
        }
        Cmd::Pair { tracker, gateway, key, new_key } => {
            let choice = match (key, new_key) {
                (Some(k), _) => KeyChoice::Explicit(pair::parse_key(&k)?),
                (None, true) => KeyChoice::New(random_key()),
                (None, false) => KeyChoice::Existing,
            };
            pair_files(&tracker, &gateway, choice)?;
        }
        Cmd::Edit { file, role, port } => {
            if !file.exists() {
                let role = match role {
                    Some(r) => r.into(),
                    None => ask_role(&file)?,
                };
                let id = create_profile(&file, role, None)?;
                println!("{} didn't exist; created a basic {role:?} profile (device_id {id})", file.display());
            }
            edit(&file, port.as_deref())?
        }
        Cmd::Show { file } => print_paths(&load(&file)?),
        Cmd::Flash { file, firmware, port } => {
            let cfg = load(&file)?;
            flash::flash_all(&cfg, firmware.as_deref(), port.as_deref())?;
            println!("firmware and config flashed");
        }
        Cmd::FlashConfig { file, port } => {
            flash::flash_config(&load(&file)?, port.as_deref())?;
            println!("config flashed");
        }
        Cmd::Init { role, device_id, out, force } => {
            check_overwrite(&out, force)?;
            let id = create_profile(&out, role.into(), device_id)?;
            println!("wrote {} (device_id {id})", out.display());
        }
        Cmd::Pack { file, out } => {
            let cfg = load(&file)?;
            let mut buf = [0u8; pc::MAX_CONFIG];
            let n = cfg.encode(&mut buf).map_err(|e| anyhow::anyhow!("packing: {e}"))?;
            write_secret(&out, &buf[..n])?;
            println!("wrote {} ({n} bytes)", out.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_role_follows_file_name() {
        assert_eq!(suggested_role(Path::new("tracker.toml")), Role::Tracker);
        assert_eq!(suggested_role(Path::new("dir/Home-Gateway.toml")), Role::Gateway);
        assert_eq!(suggested_role(Path::new("bike.toml")), Role::Tracker);
    }

    #[test]
    fn created_profile_is_valid_and_basic() {
        for role in [Role::Tracker, Role::Gateway] {
            let path = std::env::temp_dir().join(format!("lt-profile-{}-{role:?}.toml", std::process::id()));
            let id = create_profile(&path, role, Some(42)).unwrap();
            let cfg = load(&path);
            std::fs::remove_file(&path).unwrap();
            let cfg = cfg.unwrap();
            assert_eq!((id, cfg.device_id), (42, 42));
            assert_eq!(cfg.device_key.is_some(), role == Role::Tracker);
            // Basic profile: every optional path off, BLE status on.
            let p = cfg.paths();
            assert_eq!(p.ble_status, PathPlan::Enabled);
            assert!(matches!(p.lora, PathPlan::Disabled(_)) && matches!(p.wifi, PathPlan::Disabled(_)));
        }
    }
}
