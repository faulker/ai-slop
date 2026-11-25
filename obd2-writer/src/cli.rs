use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "obd2", about = "OBD2 read/write tool for a 2023 Toyota Tacoma and a 2023 BMW R 1250 GS Adventure (K51)")]
pub struct Cli {
    /// Serial port path (if omitted, lists available devices for selection)
    #[arg(short, long)]
    pub port: Option<String>,

    /// Baud rate
    #[arg(short, long, default_value_t = 115200)]
    pub baud_rate: u32,

    /// Response timeout in milliseconds
    #[arg(short, long, default_value_t = 3000)]
    pub timeout: u64,

    /// Enable verbose protocol logging
    #[arg(short, long)]
    pub verbose: bool,

    /// Which vehicle is connected. Changes how `ecus` looks for modules.
    #[arg(long, value_enum, global = true, default_value_t = Vehicle::ToyotaTacoma)]
    pub vehicle: Vehicle,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Vehicle {
    /// 2023 Toyota Tacoma: 11-bit CAN, modules at 7xx
    ToyotaTacoma,
    /// 2023 BMW R 1250 GS Adventure: D-CAN with extended addressing ("6F1:<module>")
    BmwK51,
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
    Ecus {
        /// Probe every 11-bit address in a range instead of the known Toyota list
        /// (e.g., "700-7FF"). Use this on a vehicle whose module addresses are unknown.
        #[arg(long, value_name = "RANGE", num_args = 0..=1, default_missing_value = "700-7FF")]
        sweep: Option<String>,
    },

    /// Read-only DID snapshot of one ECU, for before/after comparison with `diff`
    Snapshot {
        /// Target ECU header (e.g., "7E0"). Required with --scan.
        #[arg(long)]
        ecu: Option<String>,

        /// Discover DIDs in this hex range (e.g., "0000-FFFF") and record their values
        #[arg(long, value_name = "RANGE", conflicts_with = "from")]
        scan: Option<String>,

        /// Re-read exactly the DIDs listed in a previous snapshot file
        #[arg(long, value_name = "FILE", conflicts_with = "scan")]
        from: Option<String>,

        /// Label stored in the snapshot (e.g., "before", "after")
        #[arg(short, long, default_value = "snapshot")]
        label: String,

        /// Output JSON file
        #[arg(short, long)]
        output: String,
    },

    /// Passively capture CAN broadcast traffic (no requests sent) for `diff`
    Monitor {
        /// Seconds to listen per 0x100-wide ID window (8 windows total)
        #[arg(default_value_t = 3)]
        seconds: u64,

        /// Label stored in the capture (e.g., "before", "after")
        #[arg(short, long, default_value = "capture")]
        label: String,

        /// Output JSON file
        #[arg(short, long)]
        output: String,
    },

    /// Full read-only before/after recon over a single connection:
    /// sweep, baseline, capture, wait for you to press buttons, re-read, diff
    Recon {
        /// Comma-separated ECU addresses (e.g., "7E0,787"). Omit to sweep 700-7FF and use all responders.
        #[arg(long)]
        ecus: Option<String>,

        /// Comma-separated DID ranges to scan on each ECU
        #[arg(long, default_value = "0000-1FFF,F000-FFFF")]
        ranges: String,

        /// Seconds per passive monitor window (0 to skip monitoring)
        #[arg(long, default_value_t = 3)]
        monitor_seconds: u64,

        /// Directory for the JSON output files
        #[arg(short, long, default_value = "recon")]
        out_dir: String,
    },

    /// Diff two snapshot files or two monitor capture files
    Diff {
        /// Baseline JSON file
        before: String,
        /// Comparison JSON file
        after: String,
    },

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

    /// BMW motorcycle tools (read-only for now)
    Bmw {
        #[command(subcommand)]
        action: BmwAction,
    },

    /// Start interactive shell
    Shell,

    /// Launch the TUI dashboard
    Tui,
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

#[derive(Subcommand)]
pub enum BmwAction {
    /// Find modules by probing extended addresses with TesterPresent (read-only)
    Sweep {
        /// Address range as hex (e.g., "00-FF")
        #[arg(default_value = "00-FF")]
        range: String,
    },

    /// Read part numbers, versions and VIN from one module (read-only)
    Identify {
        /// Module extended address as hex (e.g., "60")
        module: String,
    },

    /// List the known settings in plain language, with risk and verified status
    Explain {
        /// Only show settings on this module (hex address)
        #[arg(long)]
        module: Option<String>,

        /// Connect and show current values for mapped settings
        #[arg(long)]
        read: bool,

        /// Settings file (default: bmw_k51.toml next to the binary or in the working dir)
        #[arg(long)]
        file: Option<String>,
    },
}
