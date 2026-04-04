# OBD2 Read/Write Tool for 2023 Toyota Tacoma

> **WARNING: Use at your own risk. Writing incorrect data to your vehicle's ECU can permanently damage its computer, disable safety systems, or brick modules beyond repair. The authors assume no liability for any damage caused by use of this tool.**

A Rust CLI application for reading from and writing to a 2023 Toyota Tacoma's ECUs via an OBDLink MX+ Bluetooth scanner. Includes a full-featured TUI dashboard for live data, settings editing, scanning, and backup/restore.

## Hardware Requirements

- **OBDLink MX+** OBD2 Bluetooth Scanner (genuine, STN-based — reports `ELM327 v1.4b` via ATI for compatibility)
- **2023 Toyota Tacoma** (3rd gen, ICE)
- macOS with Bluetooth

## Prerequisites

1. **Pair the OBDLink MX+** with your Mac via System Settings → Bluetooth. Press the Connect button on the device to enable pairing.
2. The device will appear as a serial port (typically `/dev/cu.OBDLink_MXp-SPPDev`).
3. Vehicle ignition must be ON (engine running or accessory mode).

## Setup

```bash
# Build
cargo build --release

# Run tests
cargo test
```

## Usage

### TUI Dashboard

```bash
cargo run -- tui
```

The TUI provides a full-screen terminal interface with six tabs:

| Tab | Key | Description |
|-----|-----|-------------|
| **Dashboard** | `1` | Live PID/DID gauges with configurable polling interval |
| **Settings** | `2` | Browse and toggle writable vehicle settings |
| **DTCs** | `3` | View and clear diagnostic trouble codes |
| **Scans** | `4` | ECU discovery and DID range scanning (UDS + KWP2000) |
| **Backup** | `5` | Backup all settings, restore individual values |
| **Raw** | `6` | Send AT commands, raw hex, and special diagnostic commands |

#### TUI Controls

| Key | Action |
|-----|--------|
| `1`–`6` | Switch tabs |
| `Tab` / `Shift+Tab` | Next / previous tab |
| `q` | Quit |
| `Ctrl+C` | Force quit (works in any context) |

**Dashboard tab:**
- `Enter` — Open PID/DID picker (type to filter, supports both standard OBD2 PIDs and Toyota enhanced DIDs)
- `Space` — Pause / resume live polling
- `+` / `-` — Adjust poll interval (default 500ms)
- `d` — Remove selected PID from display
- `Up` / `Down` — Navigate PID list

**Settings tab:**
- `t` — **Toggle** setting (cycles between named values, e.g. ON/OFF)
- `Enter` — Edit selected DID (enter new hex value manually)
- `d` — Dry-run preview (validates without writing)
- `r` — Read current value from ECU
- `R` — Read all writable DIDs
- After any write, auto-reads the value back to confirm

**DTCs tab:**
- `r` — Refresh (fetch stored DTCs)
- `c` — Clear all DTCs (with confirmation)

**Scans tab:**
- `e` — Run ECU scan (discovers responding modules)
- `s` — Start DID range scan
- `m` — **Cycle scan mode**: UDS 0x22, KWP 0x21, KWP 0x1A
- `i` — Edit scan parameters (ECU address, DID range)
- `w` — Toggle writable-DID testing
- `Enter` — Select found ECU as DID scan target
- `Esc` — Cancel running scan
- `Tab` — Switch between ECU and DID scan panes

**Backup tab:**
- `a` — Backup all writable DIDs (grouped by ECU, supports KWP + UDS)
- `r` — Restore selected backup to ECU (with confirmation)

**Raw tab:**
- Type commands and press `Enter` to send
- `Esc` — Exit input mode (allows tab navigation)
- Any key — Re-enter input mode
- `F5` — Clear output
- `F6` — Save output to `raw-output.txt`
- `Up` / `Down` — Command history

Special Raw commands:
| Prefix | Description |
|--------|-------------|
| `sec <ECU> <level>` | Request security access seed with ISO-TP reassembly |
| `unlock <ECU> <level>` | Automated security unlock (tries common algorithms) |
| `key <ECU> <level+key>` | Send security key manually |
| `dbg <cmd>` | Send command and show raw hex dump of response |

### One-shot Commands

```bash
# Test connection
cargo run -- connect

# Read standard OBD2 PIDs
cargo run -- read rpm
cargo run -- read coolant_temp
cargo run -- read speed

# Read Toyota-specific enhanced DID
cargo run -- read-enhanced 0100

# Interactive PID/DID browsers (fuzzy search)
cargo run -- browse            # pick a standard PID
cargo run -- browse-enhanced   # pick a Toyota DID

# Scan for responding ECUs
cargo run -- ecus

# List/clear DTCs
cargo run -- dtc list
cargo run -- dtc clear

# Backup all configured DID values
cargo run -- backup-all

# Discover DIDs by brute-force scanning an ECU
cargo run -- scan 7C0 00-FF               # scan Combination Meter
cargo run -- scan 750 B000-B1FF           # scan BCM range
cargo run -- scan 750 --test-writable     # also check if DIDs are writable

# Change UDS session
cargo run -- session extended

# Write to a DID (requires --confirm flag)
cargo run -- write A7 00 --ecu 7C0 --confirm   # disable seatbelt chime
```

### Interactive Shell

```bash
cargo run -- shell
```

## Discovered ECUs

The 2023 Tacoma has a **security gateway** that blocks direct UDS access to powertrain ECUs. Broadcast OBD2 requests (via 0x7DF) are forwarded, but direct addressing to the ECM is blocked.

### Responding ECUs

| Address | Name | Protocol | Notes |
|---------|------|----------|-------|
| **0x7C0** | **Combination Meter** | **KWP2000** | Seatbelt chime, instrument cluster settings |
| 0x7C4 | HVAC | Unknown | Responds to TesterPresent |
| 0x7B0 | ABS/VSC | Unknown | Responds to TesterPresent |
| 0x780 | SRS (Airbag) | Unknown | Responds (NRC 0x12) |
| 0x790 | Parking Assist | Unknown | Responds to TesterPresent |
| 0x701 | TCM #2 (Transmission) | Unknown | Responds to TesterPresent |

### Non-responding ECUs (gateway blocked)

| Address | Name | Notes |
|---------|------|-------|
| 0x7E0 | ECM (Engine) | Responds to broadcast only, blocked for direct addressing |
| 0x7E1 | TCM (Transmission) | Gateway blocked |
| 0x750 | BCM (Body/Gateway) | Uses KWP2000 with sub-addressing, did not respond |
| 0x760 | Gateway | No response |
| 0x7A0 | EPS (Power Steering) | No response |

## Protocol Details

### Combination Meter (0x7C0) — KWP2000

The Combination Meter does **NOT** support UDS services (0x22/0x2E). It uses KWP2000:

| Service | Name | Status |
|---------|------|--------|
| **0x21** | ReadDataByLocalIdentifier | Works — 21 identifiers found |
| **0x3B** | WriteDataByLocalIdentifier | Works for writable IDs |
| 0x1A | ReadEcuIdentification | Works (part number "4W70") |
| 0x10 | DiagnosticSessionControl | Only default session (0x01) |
| 0x27 | SecurityAccess | Level 0x61 returns 6-byte seed |
| 0x22 | UDS ReadDataByIdentifier | **Not supported** (NRC 0x11) |

### Verified Settings

| Setting | ECU | Local ID | Values | Protocol |
|---------|-----|----------|--------|----------|
| **Seatbelt Warning Chime** | 0x7C0 | **0xA7** | `C0` = ON, `00` = OFF | KWP 0x21/0x3B |

### CAN Bus

- **Vehicle protocol:** ISO 15765-4 CAN, 11-bit addressing, 500 kbps
- **Communication:** UDS/KWP2000 over ISO-TP (ISO 15765-2) via ELM327 AT commands
- **Gateway:** 2020+ Toyota security gateway blocks direct diagnostic access to powertrain ECUs

## Toyota Enhanced DIDs

Toyota-specific DIDs are defined in `toyota_dids.toml`. Each entry specifies:
- `protocol` — `"uds"` (service 0x22/0x2E) or `"kwp"` (service 0x21/0x3B)
- `writable` — whether the Settings tab allows writing
- `values` — named value map for toggle display (e.g. `{00 = "OFF", C0 = "ON"}`)
- `data_length`, `min_value`, `max_value` — write safety constraints
- `category` — grouping for display

The TOML currently includes:
- **ECM (0x7E0):** Engine coolant temp, intake air temp, RPM, throttle, load, MAF, fuel pressure, ignition timing, battery voltage
- **TCM (0x7E1):** ATF pan temp, ATF post-converter temp (community verified)
- **Combination Meter (0x7C0, KWP):** Seatbelt warning chime — **verified and writable**
- **BCM (0x750):** 9 placeholder entries marked `[NOT VERIFIED]` with `writable = false` — real DID addresses unknown
- **Diagnostic:** Active session, VIN, ECU software version

## Write Operations

For **KWP2000 ECUs** (e.g., Combination Meter), writes use service 0x3B (WriteDataByLocalIdentifier). The write may return NRC 0x78 (responsePending) before the positive response — the tool handles this automatically.

For **UDS ECUs**, writes use service 0x2E (WriteDataByIdentifier) with automatic session management, backup, verification, and rollback on failure.

Both the CLI (`--dry-run`) and TUI (`d` key) support dry-run mode.

## Architecture

```
src/
├── main.rs                 # Entry point, CLI routing
├── cli.rs                  # Clap CLI definition
├── shell.rs                # Interactive REPL
├── browse.rs               # Fuzzy-select PID/DID browsers
├── error.rs                # Unified error type with UDS NRC decoding
│
├── tui/                    # TUI dashboard (ratatui + crossterm)
│   ├── mod.rs              # Terminal setup/teardown, main event loop
│   ├── app.rs              # App state, tab management, render dispatch
│   ├── elm_actor.rs        # Actor pattern for non-blocking Elm327 communication
│   ├── event.rs            # Key event → action mapping
│   ├── screens/            # Tab screen implementations
│   │   ├── dashboard.rs    # Live PID gauges with configurable polling
│   │   ├── settings.rs     # Writable DID editor with toggle and confirmation
│   │   ├── dtc.rs          # DTC list/clear
│   │   ├── scans.rs        # ECU scan + DID range scan (UDS/KWP modes)
│   │   ├── backup.rs       # Backup/restore management
│   │   └── raw.rs          # Command passthrough with history, debug, security
│   └── widgets/            # Reusable TUI components
│       ├── gauge.rs        # Horizontal bar gauge
│       ├── status_bar.rs   # Connection info display
│       ├── confirm.rs      # Modal yes/no dialog
│       └── pid_picker.rs   # Filterable PID/DID selection list
│
├── obd/                    # Standard OBD2
│   ├── pid.rs              # PID definitions and reading (Mode 01)
│   └── dtc.rs              # DTC read/clear (Mode 03/04)
│
├── toyota/                 # Toyota-specific
│   ├── enhanced_pids.rs    # Enhanced DIDs, TOML config loader (UDS + KWP)
│   ├── write_safety.rs     # Verified writes with whitelist, backup, rollback
│   ├── did_scan.rs         # DID range discovery
│   ├── ecu_scan.rs         # ECU discovery via TesterPresent
│   ├── backup.rs           # JSON backup store
│   ├── bcm.rs              # BCM write operations
│   └── tpms.rs             # TPMS sensor registration
│
├── protocol/               # Protocol implementations
│   ├── uds.rs              # UDS services (0x10, 0x22, 0x2E, 0x27, 0x3E)
│   └── isotp.rs            # ISO-TP multi-frame reassembly
│
└── transport/              # Hardware communication
    ├── elm327.rs            # ELM327 AT commands and response parsing
    └── serial.rs            # Serial port connection and port selection
```

### Data Flow

```
CLI / Shell / TUI → Elm327 (AT commands over serial) → OBDLink MX+ → CAN bus → Vehicle ECU
```

The TUI uses an **actor pattern**: a dedicated tokio task owns the `Elm327` connection exclusively. The UI sends commands via async channels and polls for responses, keeping the interface responsive during slow ECU operations.

## Known Issues

- **Bluetooth disconnect on macOS:** Closing the serial port triggers macOS to terminate the RFCOMM channel. Mitigated by using `process::exit(0)` to avoid explicit serial port closure, but may still occur in some situations.
- **Multi-frame TX:** The OBDLink MX+ supports multi-frame ISO-TP for outgoing messages with `ATCAF1`, but `ATCAF0` (manual framing) has issues with flow control. Keep auto-formatting enabled.
- **Security gateway:** The 2023 Tacoma's security gateway blocks direct UDS diagnostic access to powertrain ECUs (ECM, TCM). Only broadcast OBD2 requests work for these modules.

## Debugging

Enable verbose logging to see all protocol exchanges:

```bash
cargo run -- -v connect
```

Or set the environment variable:

```bash
RUST_LOG=debug cargo run -- connect
```

## Tested On

- 2023 Toyota Tacoma (3rd gen, ICE)
- OBDLink MX+ (genuine, firmware updated)
- macOS (Darwin 25.4.0)
