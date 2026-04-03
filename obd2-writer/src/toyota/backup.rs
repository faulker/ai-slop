use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupEntry {
    pub did: String,
    pub ecu: String,
    pub original_data: String,
    pub timestamp: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BackupStore {
    pub backups: HashMap<String, BackupEntry>,
}

impl BackupStore {
    fn backup_path() -> PathBuf {
        PathBuf::from("obd2_backups.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::backup_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| Error::Config(format!("failed to read backup file: {}", e)))?;
        serde_json::from_str(&content)
            .map_err(|e| Error::Config(format!("failed to parse backup file: {}", e)))
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("failed to serialize backups: {}", e)))?;
        std::fs::write(Self::backup_path(), content)
            .map_err(|e| Error::Config(format!("failed to write backup file: {}", e)))?;
        Ok(())
    }

    /// Record the original value of a DID before writing.
    /// Only saves the first backup per ECU+DID to preserve the true original.
    pub fn record(&mut self, ecu: &str, did: u16, original_data: &[u8]) -> Result<()> {
        let key = format!("{}:{:04X}", ecu, did);
        if self.backups.contains_key(&key) {
            println!("Backup already exists for {} — preserving original value.", key);
            return Ok(());
        }
        let data_hex = original_data
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" ");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.backups.insert(
            key,
            BackupEntry {
                did: format!("{:04X}", did),
                ecu: ecu.to_string(),
                original_data: data_hex,
                timestamp: format!("{}", now),
            },
        );
        Ok(())
    }

    pub fn get(&self, ecu: &str, did: u16) -> Option<&BackupEntry> {
        let key = format!("{}:{:04X}", ecu, did);
        self.backups.get(&key)
    }

    pub fn list(&self) -> Vec<(&String, &BackupEntry)> {
        let mut entries: Vec<_> = self.backups.iter().collect();
        entries.sort_by_key(|(k, _)| (*k).clone());
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_get() {
        let mut store = BackupStore::default();
        store.record("7E0", 0xF190, &[0x01, 0x02]).unwrap();

        let entry = store.get("7E0", 0xF190).unwrap();
        assert_eq!(entry.did, "F190");
        assert_eq!(entry.ecu, "7E0");
        assert_eq!(entry.original_data, "01 02");
    }

    #[test]
    fn test_record_preserves_original() {
        let mut store = BackupStore::default();
        store.record("7E0", 0xF190, &[0x01]).unwrap();
        store.record("7E0", 0xF190, &[0xFF]).unwrap();

        let entry = store.get("7E0", 0xF190).unwrap();
        assert_eq!(entry.original_data, "01");
    }

    #[test]
    fn test_list_sorted() {
        let mut store = BackupStore::default();
        store.record("7E0", 0xF190, &[0x01]).unwrap();
        store.record("750", 0x0100, &[0x02]).unwrap();

        let list = store.list();
        assert_eq!(list.len(), 2);
        assert!(list[0].0 < list[1].0);
    }
}
