//! Flash: the device config written by `lora-tracker flash-config`, and the
//! boot counter that keeps report sequence numbers unique across reboots.

use esp_storage::FlashStorage;
use tracker_proto::Error;
use tracker_proto::config::{CONFIG_FLASH_OFFSET, Config, MAX_CONFIG};

/// First sector of the `nvs` partition. This firmware doesn't use ESP-IDF's
/// NVS, so it keeps its boot counter there (see `partitions.csv`).
const BOOT_COUNTER_OFFSET: u32 = 0x9000;
const BOOT_MAGIC: [u8; 4] = *b"LTBC";

/// Reads and decodes the config partition, or says briefly why it can't
/// (the reason is shown on the OLED).
pub fn load_config(flash: &mut FlashStorage<'_>) -> Result<Config, &'static str> {
    let mut blob = [0u8; MAX_CONFIG];
    flash.read(CONFIG_FLASH_OFFSET, &mut blob).map_err(|_| "read error")?;
    if blob.iter().all(|b| *b == 0xFF) {
        return Err("blank");
    }
    Config::decode(&blob).map_err(|e| match e {
        Error::BadMagic => "not a config",
        Error::BadVersion(_) => "old version",
        _ => "corrupt",
    })
}

/// Increments and persists the boot counter, returning the new value. Report
/// sequence numbers are `boot << 16 | n`, so they never repeat under one key
/// (until 65536 boots). `None` if flash can't be written: then reports must
/// not be sent, since their nonces could repeat.
pub fn next_boot(flash: &mut FlashStorage<'_>) -> Option<u16> {
    let mut rec = [0u8; 8];
    let prev = match flash.read(BOOT_COUNTER_OFFSET, &mut rec) {
        Ok(()) if rec[..4] == BOOT_MAGIC => u32::from_le_bytes(rec[4..].try_into().unwrap()),
        _ => 0,
    };
    let next = prev.wrapping_add(1);
    rec[..4].copy_from_slice(&BOOT_MAGIC);
    rec[4..].copy_from_slice(&next.to_le_bytes());
    flash.write(BOOT_COUNTER_OFFSET, &rec).ok().map(|()| next as u16)
}
