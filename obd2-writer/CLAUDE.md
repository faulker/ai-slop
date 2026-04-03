# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
cargo build              # debug build
cargo build --release    # release build
cargo test               # run all tests
cargo test <test_name>   # run a single test
cargo run -- <command>   # run with subcommand (connect, read, write, shell, etc.)
```

Verbose protocol logging: `cargo run -- -v <command>` or `RUST_LOG=debug cargo run -- <command>`

## Architecture

Rust async CLI (tokio) for reading/writing to a 2023 Toyota Tacoma via an OBDLink MX+ Bluetooth OBD2 scanner.

### Module Layout

- **`transport/`** — Serial port connection (`serial.rs`) and ELM327 AT command interface (`elm327.rs`). All hardware communication goes through `Elm327`.
- **`protocol/`** — Protocol implementations: ISO-TP framing (`isotp.rs`) and UDS services (`uds.rs`) including DiagnosticSessionControl, ReadDataByIdentifier, WriteDataByIdentifier, SecurityAccess.
- **`obd/`** — Standard OBD2: PID definitions and reading (`pid.rs`), DTC read/clear (`dtc.rs`).
- **`toyota/`** — Toyota-specific: enhanced DIDs via Mode 22 (`enhanced_pids.rs`), BCM write operations (`bcm.rs`), TPMS (`tpms.rs`), security access (`security.rs` — currently empty).
- **`cli.rs`** — Clap-derived CLI definition with subcommands.
- **`shell.rs`** — Interactive REPL (rustyline-based) with its own command parser.
- **`error.rs`** — Unified error type with UDS NRC decoding.

### Data Flow

CLI/Shell → `Elm327` (AT commands over serial) → ELM327 device → CAN bus → Vehicle ECU

All ECU communication flows through `Elm327::send_command()`. UDS operations in `protocol/uds.rs` build hex command strings and parse responses. Toyota-specific modules use UDS primitives for enhanced reads and writes.

### Key Design Points

- Write operations require `--confirm` flag and auto-manage session transitions (enter extended session → write → return to default).
- Toyota enhanced DIDs are defined in `toyota_dids.toml` (TOML config, loaded at runtime).
- The interactive shell maintains connection state and supports all operations available as one-shot CLI commands.
- Vehicle protocol: ISO 15765-4 CAN, 11-bit addressing, 500 kbps. ECM at 0x7E0/0x7E8.
