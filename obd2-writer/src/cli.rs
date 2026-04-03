use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "obd2", about = "OBD2 read/write tool for 2023 Toyota Tacoma")]
pub struct Cli {
    /// Serial port path (if omitted, lists available devices for selection)
    #[arg(short, long)]
    pub port: Option<String>,

    /// Baud rate
    #[arg(short, long, default_value_t = 115200)]
    pub baud_rate: u32,

    /// Response timeout in milliseconds
    #[arg(short, long, default_value_t = 2000)]
    pub timeout: u64,

    /// Enable verbose protocol logging
    #[arg(short, long)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Connect to OBDLink MX+ and print device info
    Connect,

    /// Read a standard OBD2 PID
    Read {
        /// PID name (e.g., "rpm", "speed", "coolant_temp") or hex (e.g., "0C")
        pid: String,
    },

    /// Read a Toyota-specific enhanced DID (Mode 22)
    ReadEnhanced {
        /// DID as hex (e.g., "0100")
        did: String,

        /// Target ECU header (default: 7E0)
        #[arg(long, default_value = "7E0")]
        ecu: String,
    },

    /// Read or clear diagnostic trouble codes
    Dtc {
        #[command(subcommand)]
        action: DtcAction,
    },

    /// Manage UDS diagnostic session
    Session {
        /// Session type: default, extended, programming
        session_type: String,
    },

    /// Write data to an ECU via UDS WriteDataByIdentifier.
    /// WARNING: Writing incorrect data can damage the ECU. Use --confirm to proceed.
    Write {
        /// DID as hex (e.g., "F190")
        did: String,

        /// Data to write as hex (e.g., "01")
        data: String,

        /// Target ECU header (default: 7E0)
        #[arg(long, default_value = "7E0")]
        ecu: String,

        /// Required flag to confirm the write operation
        #[arg(long)]
        confirm: bool,

        /// Dry run: read the DID and validate without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Restore a previously backed-up DID value
    Restore {
        /// DID as hex (e.g., "F190")
        did: String,

        /// Target ECU header (default: 7E0)
        #[arg(long, default_value = "7E0")]
        ecu: String,

        /// Required flag to confirm the restore operation
        #[arg(long)]
        confirm: bool,
    },

    /// List all backed-up DID values
    Backups,

    /// Backup all configured DID values from the vehicle
    BackupAll,

    /// Scan for responding ECUs on the CAN bus
    Ecus,

    /// Interactive PID browser — select and read a standard OBD2 PID
    Browse,

    /// Interactive DID browser — select and read a Toyota enhanced DID
    BrowseEnhanced,

    /// Scan a DID range on an ECU to discover valid DIDs
    Scan {
        /// Target ECU header (e.g., "750" for BCM, "7E0" for ECM)
        ecu: String,

        /// DID range as hex (e.g., "B000-B1FF"). Omit to scan all Toyota BCM ranges.
        range: Option<String>,

        /// Test if discovered DIDs are writable (writes current value back)
        #[arg(long)]
        test_writable: bool,

        /// Save results to a TOML file
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Start interactive shell
    Shell,
}

#[derive(Subcommand)]
pub enum DtcAction {
    /// List stored DTCs
    List,
    /// Clear all DTCs (requires --confirm)
    Clear {
        /// Required flag to confirm clearing DTCs
        #[arg(long)]
        confirm: bool,
    },
}
