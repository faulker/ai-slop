# CLAUDE.md

Multi-path GPS tracker for Heltec WiFi LoRa 32 boards: V2 (ESP32 + SX1276, the user's hardware) and V3 (ESP32-S3 + SX1262). See `README.md` for commands and `docs/protocol.md` for wire/config formats.

- Host workspace (`proto`, `fwlogic` = hardware-free firmware logic, `cli` = the `lora-tracker` setup/pair/flash command, `server` = `lora-tracker-server`, axum + bundled SQLite): `cargo test`.
- `firmware/` is a separate Cargo project (Xtensa, esp toolchain, esp-hal + embassy), excluded from the workspace. Build: `cd firmware && . ~/export-esp.sh && cargo build-v2` (or `build-v3`; `clippy-v2`/`clippy-v3`). The board is a `board-v2`/`board-v3` feature; pins are in `main.rs`, radio chip in `radio.rs`, battery in `battery.rs`. Check both boards build after firmware changes. It depends on `proto` and `fwlogic` by path.
- Put anything testable without hardware in `fwlogic` (also `no_std`, no allocator), not `firmware/`.
- `proto` must stay `no_std` with no allocator.
- Every uplink is optional. `Config::paths()` in `proto/src/config.rs` is the only place that decides which paths start. Never let one path's absence or failure block another.
- No personal ids, keys or SSIDs as defaults anywhere. They come from the user's `device.toml`.
- `firmware/partitions.csv` and `CONFIG_FLASH_OFFSET`/`CONFIG_PARTITION_SIZE` must agree (a CLI test checks this).
