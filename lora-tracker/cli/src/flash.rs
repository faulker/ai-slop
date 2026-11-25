//! Flashing through [espflash](https://github.com/esp-rs/espflash).
//!
//! Firmware and config are written separately: the config lives in its own
//! `ltcfg` partition, so changing settings never needs a firmware rebuild and
//! reflashing firmware never wipes settings.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use tracker_proto::config as pc;

use crate::secret::write_secret;

/// The Heltec WiFi LoRa 32 revisions the firmware builds for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    /// ESP32 + SX1276.
    V2,
    /// ESP32-S3 + SX1262.
    V3,
}

impl Board {
    /// The Rust target the firmware is built for on this board.
    fn target(self) -> &'static str {
        match self {
            Board::V2 => "xtensa-esp32-none-elf",
            Board::V3 => "xtensa-esp32s3-none-elf",
        }
    }

    /// The cargo alias in `firmware/.cargo/config.toml` that builds for this board.
    fn build_cmd(self) -> &'static str {
        match self {
            Board::V2 => "cargo build-v2",
            Board::V3 => "cargo build-v3",
        }
    }
}

/// Default firmware ELF for `board`, relative to this repo checkout.
pub fn default_firmware(board: Board) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../firmware/target")
        .join(board.target())
        .join("release/lora-tracker-firmware")
}

/// The board for the `Chip type:` line of `espflash board-info` output.
fn parse_chip(board_info: &str) -> Result<Board> {
    let chip = board_info
        .lines()
        .find_map(|l| l.trim().strip_prefix("Chip type:"))
        .and_then(|rest| rest.split_whitespace().next())
        .context("espflash board-info printed no chip type")?;
    match chip {
        "esp32" => Ok(Board::V2),
        "esp32s3" => Ok(Board::V3),
        other => bail!("unsupported chip {other}; this firmware runs on Heltec WiFi LoRa 32 V2 (esp32) or V3 (esp32s3)"),
    }
}

/// Asks the connected board which chip it has.
pub fn detect_board(port: Option<&str>) -> Result<Board> {
    let mut cmd = Command::new("espflash");
    cmd.arg("board-info");
    if let Some(p) = port {
        cmd.args(["--port", p]);
    }
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("espflash not found; install it with `cargo install espflash`")
        }
        Err(e) => return Err(e).context("running espflash board-info"),
    };
    if !out.status.success() {
        bail!("espflash board-info failed; is the board plugged in? Try --port");
    }
    parse_chip(&String::from_utf8_lossy(&out.stdout))
}

/// The partition table that defines where `ltcfg` lives.
pub fn partition_table() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../firmware/partitions.csv")
}

/// espflash arguments that write only the config blob.
pub fn config_args(bin: &Path, port: Option<&str>) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["write-bin".into()];
    if let Some(p) = port {
        a.extend(["--port".into(), p.into()]);
    }
    a.push(format!("{:#x}", pc::CONFIG_FLASH_OFFSET).into());
    a.push(bin.into());
    a
}

/// espflash arguments that write the firmware with our partition table.
pub fn firmware_args(elf: &Path, partitions: &Path, port: Option<&str>) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["flash".into(), "--partition-table".into(), partitions.into()];
    if let Some(p) = port {
        a.extend(["--port".into(), p.into()]);
    }
    a.push(elf.into());
    a
}

/// Runs espflash, with a clear message if it isn't installed.
fn run_espflash(args: &[OsString]) -> Result<()> {
    let status = match Command::new("espflash").args(args).status() {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("espflash not found; install it with `cargo install espflash`")
        }
        Err(e) => return Err(e).context("running espflash"),
    };
    if !status.success() {
        bail!("espflash failed ({status}); is the board plugged in? Try --port, or hold PRG while pressing RST");
    }
    Ok(())
}

/// Packs `cfg` to a temporary owner-only file and writes it to the board.
pub fn flash_config(cfg: &pc::Config, port: Option<&str>) -> Result<()> {
    let mut buf = [0u8; pc::MAX_CONFIG];
    let n = cfg.encode(&mut buf).map_err(|e| anyhow::anyhow!("packing config: {e}"))?;
    let tmp = std::env::temp_dir().join(format!("lora-tracker-config-{}.bin", std::process::id()));
    write_secret(&tmp, &buf[..n])?;
    let result = run_espflash(&config_args(&tmp, port));
    // Remove the key-bearing temp file whether or not flashing worked.
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Writes the firmware ELF, then the config. Without `elf`, detects the
/// board and uses the matching build from `firmware/`.
pub fn flash_all(cfg: &pc::Config, elf: Option<&Path>, port: Option<&str>) -> Result<()> {
    let elf = match elf {
        Some(e) => e.to_path_buf(),
        None => {
            let board = detect_board(port)?;
            println!("detected Heltec {board:?}");
            let e = default_firmware(board);
            if !e.exists() {
                bail!(
                    "no {board:?} firmware at {}; build it with `cd firmware && . ~/export-esp.sh && {}`, or pass --firmware <ELF>",
                    e.display(),
                    board.build_cmd()
                );
            }
            e
        }
    };
    if !elf.exists() {
        bail!("firmware not found at {}", elf.display());
    }
    let elf = elf.as_path();
    run_espflash(&firmware_args(elf, &partition_table(), port))?;
    flash_config(cfg, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_args_target_the_ltcfg_partition() {
        let a = config_args(Path::new("c.bin"), Some("/dev/cu.usb1"));
        assert_eq!(a, ["write-bin", "--port", "/dev/cu.usb1", "0xe000", "c.bin"].map(OsString::from));
        let a = config_args(Path::new("c.bin"), None);
        assert_eq!(a, ["write-bin", "0xe000", "c.bin"].map(OsString::from));
    }

    #[test]
    fn firmware_args_use_partition_table() {
        let a = firmware_args(Path::new("fw"), Path::new("p.csv"), None);
        assert_eq!(a, ["flash", "--partition-table", "p.csv", "fw"].map(OsString::from));
    }

    #[test]
    fn board_info_chip_picks_the_build() {
        let v2 = "Chip type:         esp32 (revision v1.0)\nFlash size:        8MB\n";
        assert_eq!(parse_chip(v2).unwrap(), Board::V2);
        assert_eq!(parse_chip("Chip type:         esp32s3 (revision v0.2)").unwrap(), Board::V3);
        assert!(parse_chip("Chip type: esp32c3").is_err());
        assert!(parse_chip("nothing").is_err());
        assert!(default_firmware(Board::V2).ends_with("xtensa-esp32-none-elf/release/lora-tracker-firmware"));
    }

    /// The CSV and the proto constants must agree, or the firmware reads the wrong spot.
    #[test]
    fn partition_table_matches_proto_constants() {
        let csv = std::fs::read_to_string(partition_table()).unwrap();
        let row = csv
            .lines()
            .find(|l| l.trim_start().starts_with("ltcfg"))
            .expect("ltcfg row");
        let cols: Vec<&str> = row.split(',').map(str::trim).collect();
        let parse = |s: &str| u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap();
        assert_eq!(parse(cols[3]), pc::CONFIG_FLASH_OFFSET);
        assert_eq!(parse(cols[4]) as usize, pc::CONFIG_PARTITION_SIZE);
    }
}
