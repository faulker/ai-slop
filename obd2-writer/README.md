# OBD2 Read/Write Tool for 2023 Toyota Tacoma

> **WARNING: Use at your own risk. Writing incorrect data to your vehicle's ECU can permanently damage its computer, disable safety systems, or brick modules beyond repair. The authors assume no liability for any damage caused by use of this tool.**

A Rust CLI application for reading from and writing to a 2023 Toyota Tacoma's ECUs via an OBDLink MX+ Bluetooth scanner.

## Hardware Requirements

- **OBDLink MX+** OBD2 Bluetooth Scanner
- **2023 Toyota Tacoma** (3rd gen, ICE)
- macOS with Bluetooth

## Prerequisites

1. **Pair the OBDLink MX+** with your Mac via System Settings → Bluetooth. Press the Connect button on the device to enable pairing.
2. The device will appear as a serial port (typically `/dev/tty.OBDLink_MXp-SPPDev`).
3. Vehicle ignition must be ON (engine running or accessory mode).

## Setup

```bash
# Build
cargo build --release

# Run tests
cargo test
```

## Usage

### One-shot commands

```bash
# Test connection
cargo run -- connect

# Read standard OBD2 PIDs
cargo run -- read rpm
cargo run -- read coolant_temp
cargo run -- read speed

# Read Toyota-specific enhanced DID
cargo run -- read-enhanced 0100

# List/clear DTCs
cargo run -- dtc list
cargo run -- dtc clear

# Change UDS session
cargo run -- session extended

# Write to a DID (e.g., BCM customization)
cargo run -- write F190 01 --ecu 750
```

### Interactive shell

```bash
cargo run -- shell
```

Shell commands:
- `connect` — Initialize the OBDLink MX+
- `read <pid>` — Read a standard PID (by name or hex)
- `read-enhanced <did> [ecu]` — Read Toyota Mode 22 DID
- `monitor <pid> [interval_ms]` — Continuously read a PID
- `dtc [list|clear]` — Read or clear DTCs
- `session <default|extended|programming>` — Set diagnostic session
- `security [level]` — Perform security access handshake
- `write <did> <data> [ecu]` — Write to a DID
- `target <ecu>` — Set target ECU header
- `raw <hex>` — Send raw hex command
- `at <cmd>` — Send AT command
- `pids` — List available PIDs
- `help` — Show help
- `quit` — Exit

### Options

```
-p, --port <PORT>       Serial port path [default: /dev/tty.OBDLink_MXp-SPPDev]
-b, --baud-rate <RATE>  Baud rate [default: 115200]
-t, --timeout <MS>      Response timeout in ms [default: 2000]
-v, --verbose           Enable protocol logging
```

## Available PIDs

| Name | PID | Unit |
|------|-----|------|
| rpm | 0x0C | RPM |
| speed | 0x0D | km/h |
| coolant_temp | 0x05 | °C |
| intake_temp | 0x0F | °C |
| throttle | 0x11 | % |
| load | 0x04 | % |
| maf | 0x10 | g/s |
| fuel_level | 0x2F | % |
| battery_voltage | 0x42 | V |
| oil_temp | 0x5C | °C |
| ambient_temp | 0x46 | °C |
| timing_advance | 0x0E | ° |
| intake_pressure | 0x0B | kPa |
| baro_pressure | 0x33 | kPa |
| runtime | 0x1F | s |

## Toyota Enhanced DIDs

Toyota-specific DIDs are defined in `toyota_dids.toml`. Add community-discovered DIDs there. These use UDS Mode 22 (ReadDataByIdentifier).

## Write Operations

Writing to ECUs uses UDS WriteDataByIdentifier (0x2E). The tool automatically:
1. Sets the target ECU header
2. Enters Extended Diagnostic Session
3. Performs the write
4. Returns to default session

For operations requiring security access (UDS 0x27), the tool will prompt for the key. Toyota's seed-key algorithm is proprietary.

### Limitations

- **Full ECU reflashing** is not possible via ELM327-class devices (requires J2534 passthrough)
- **Toyota seed-key algorithm** is proprietary — manual key entry is supported
- **BCM CAN address** may be 0x750 or 0x7C0 depending on the specific module; experiment with the `target` command

## Protocol Details

- **Vehicle protocol:** ISO 15765-4 CAN, 11-bit addressing, 500 kbps
- **ECM:** Request 0x7E0, Response 0x7E8
- **Communication:** UDS (ISO 14229) over ISO-TP (ISO 15765-2) via ELM327 AT commands

## Debugging

Enable verbose logging to see all protocol exchanges:

```bash
cargo run -- -v connect
```

Or set the environment variable:

```bash
RUST_LOG=debug cargo run -- connect
```
