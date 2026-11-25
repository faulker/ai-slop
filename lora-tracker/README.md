# lora-tracker

A GPS tracker for a Heltec WiFi LoRa 32 V2 (ESP32 + SX1276) or V3 (ESP32-S3 + SX1262), US 915 MHz, that reports through every path it can reach, plus a second board acting as a home gateway. Every path is optional: if one isn't configured or its hardware is missing, the rest keep working.

Planned paths: custom encrypted LoRa to the gateway, WiFi (known networks, optionally open ones), Apple Find My beacons, a BLE status service for your phone, Meshtastic fallback packets, and LTE-M through a SIM7080G.

## Status

Milestone 1 of 8 is done (the shared `proto` crate and the `lora-tracker` setup CLI), the server half of milestone 3 is built, and the first firmware is written: OLED status pages, optional GPS with an IP-location fallback, LoRa tracker to gateway, and WiFi upload to the server. It builds for both boards. On a V2 it has been seen booting, sending on LoRa, joining WiFi, setting its clock and uploading to the server. Gateway receive and range haven't been tested yet. See `docs/roadmap.md`.

## Layout

- `proto/`: `no_std` library shared by everything. Sealed reports (`packet.rs`), the device config, path rules and flash layout (`config.rs`).
- `cli/`: the `lora-tracker` command for setup, pairing, editing and flashing.
- `fwlogic/`: `no_std` firmware logic that doesn't touch hardware (NMEA parsing, IP location, SNTP, HTTP, the OLED page text, the event log), so it's tested on the host.
- `server/`: `lora-tracker-server`, which receives reports over HTTP, stores them in SQLite and serves a map.
- `firmware/`: the ESP32-S3 firmware (a separate Cargo project for the Xtensa target), plus `partitions.csv` with the `ltcfg` config partition.
- `docs/protocol.md`: byte layouts and which config enables which path.
- `docs/roadmap.md`: milestones.

## Setup

You need a Rust toolchain ([rustup](https://rustup.rs)) and, for flashing, [espflash](https://github.com/esp-rs/espflash):

```sh
cargo install espflash
cargo install --path cli        # puts `lora-tracker` on your PATH
```

To build the firmware you also need the Xtensa Rust toolchain from [espup](https://github.com/esp-rs/espup):

```sh
cargo install espup --locked
espup install                   # writes ~/export-esp.sh
```

## Build and test

```sh
cargo build
cargo test                      # proto, fwlogic, cli, server
```

The firmware is a separate project:

```sh
cd firmware
. ~/export-esp.sh               # once per terminal
cargo build-v2                  # Heltec V2 (ESP32 + SX1276)
cargo build-v3                  # Heltec V3 (ESP32-S3 + SX1262)
```

Each lands in `firmware/target/<chip target>/release/`. `lora-tracker flash <toml>` asks the plugged-in board which chip it has (`espflash board-info`) and flashes the matching build. Not sure which board you have? `espflash board-info` says `esp32` for a V2 and `esp32s3` for a V3. V2 boards often have revision 1.0 chips; the firmware allows that (see `firmware/.cargo/esp-config.toml`).

## On the board

The OLED cycles through four pages. Press PRG to step through them; after a minute without a press they rotate every 8 seconds.

- **Stats**: role, id and UTC time, uptime and battery, GPS state, LoRa (packets sent, or received and rejected on a gateway, with RSSI), WiFi signal, and upload counts with the queue length.
- **Location**: where the board would say it is right now and where that came from: GPS, an IP estimate (`via IP ~`), or the last known position with its age.
- **Last connection**: the most recent WiFi session (network, signal, IP, last upload result) and the most recent LoRa packet.
- **Events**: the newest log lines with their ages. The same log, untruncated, goes to the USB serial port (`espflash monitor`).

With no config flashed, every page says so and shows the `flash-config` command.

GPS is optional. Wire a 9600 baud NMEA module's TX to GPIO23 on a V2 or GPIO47 on a V3 (its RX to GPIO22 / GPIO48) and it's picked up on its own. The full pin table is at the top of `firmware/src/main.rs`. Without one, or before it has a fix, the board asks [ip-api.com](https://ip-api.com) where its WiFi network's public IP is. That's city-level at best, and it's the location of the network's internet exit, not the board. These reports carry an `APPROX` flag, the map draws them as a large dashed circle, and they're left out of the track line. The lookup only happens while there's no GPS fix, at most every 30 minutes. Known issue: on the V2 tested so far this lookup times out (ip-api.com accepts the connection but never answers the board), so boards without GPS currently report no position. The clock comes from GPS, or from SNTP (`pool.ntp.org`) over WiFi.

## Setting up a pair of boards

```sh
lora-tracker setup -o tracker.toml    # pick "Tracker", answer the questions
lora-tracker setup -o gateway.toml    # pick "Gateway"; it offers to pair with tracker.toml
lora-tracker flash tracker.toml       # firmware + config, board plugged in over USB
lora-tracker flash gateway.toml
```

The wizard only asks about the paths that exist. Anything you skip is simply off, and the rest still run.

## Changing settings later

```sh
lora-tracker edit tracker.toml        # opens $EDITOR, checks the file, offers to re-flash
                                      # (creates a basic profile with fresh keys if the file doesn't exist)
lora-tracker show tracker.toml        # which paths are on, and why the others are off
lora-tracker flash-config tracker.toml
```

Settings live in their own flash partition. `flash-config` takes a few seconds and needs no rebuild, and reflashing firmware never wipes settings.

## The encryption key

Each tracker has one 32-byte key, shared with its gateway and the server.

```sh
lora-tracker pair tracker.toml gateway.toml              # keep the tracker's key (or create one)
lora-tracker pair tracker.toml gateway.toml --key <64 hex chars>   # use your own key
lora-tracker pair tracker.toml gateway.toml --new-key    # rotate to a fresh random key
```

`pair` updates both files without touching your comments, and prints the entry the server will need. Re-flash both configs afterwards. A gateway can pair with up to 4 trackers.

## The server

`lora-tracker-server` receives sealed reports from the boards over HTTP, stores them in SQLite (no separate database to install, it's bundled), and serves a live map. Run it on any always-on machine the boards can reach, like a home server or a Raspberry Pi.

Build and install it:

```sh
cargo install --path server     # puts `lora-tracker-server` on your PATH
# or just build it: cargo build --release, binary at target/release/lora-tracker-server
```

Configure it. The easy way is to run the server from the folder that holds your `tracker.toml` and `gateway.toml`. If `server.toml` doesn't exist there yet, it creates one for you. It copies each tracker's `device_key` and each gateway's `[[peers]]` into `[[device]]` entries, saves the file readable only by you, and prints the map URL. If a tracker and a gateway disagree on a key, it uses the tracker's key and prints a warning. It never overwrites an existing `server.toml`.

To write it by hand instead, start from the example and paste in the `[[device]]` entries that `lora-tracker pair` printed:

```sh
cp server/server.example.toml server.toml
chmod 600 server.toml           # it holds device keys
```

```toml
listen = "0.0.0.0:8080"
database = "tracker.db"          # relative to server.toml

[[device]]
device_id = 1
key = "<the same 64 hex chars as device_key in tracker.toml>"
name = "Truck"                   # optional, shown on the map
```

Start it:

```sh
lora-tracker-server --config server.toml    # --config defaults to ./server.toml
```

The server prints a line for every board request: stored reports, telemetry heartbeats, the board's own log lines, and anything it rejected and why (unknown device, wrong key, wrong format). The same stream, plus a table of connected devices with their latest telemetry, is at `http://<server>:8080/log`.

Boards with WiFi phone home with a telemetry heartbeat as soon as they connect and then every minute, whether or not they know where they are. Position reports are slower: every `stationary_interval_s` (10 minutes by default) unless GPS says the board is moving.

Open `http://<server>:8080/` for the map. There's no login, so anyone who can reach that port can see positions; keep it on your home network. It shows every device that has ever reported at its last known location (the last report with a position, even if it's old; IP estimates are marked approximate), when each one was last heard from, its battery, and its recent track. It refreshes every 15 seconds. A device you remove from `server.toml` stays on the map with its history, but new reports from it are rejected. Restart the server after adding devices.

Point the boards at it by adding this to `tracker.toml` and `gateway.toml`, then re-flash the configs:

```toml
[server]
url = "http://<server>:8080"
```

The WiFi and LTE paths stay off until a board has a `[server]` section (`lora-tracker show` reports this as `NoServer`). LoRa to the gateway doesn't need one. Boards post to `POST /ingest`; see `docs/protocol.md` for the HTTP details.

These `.toml` files hold keys and WiFi passwords. The CLI saves them as readable only by you. Don't share or commit them (`.gitignore` covers the default names).
