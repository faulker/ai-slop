# Protocol

## Sealed report (LoRa, WiFi, LTE)

One wire format for every custom path. Each tracker has one key, shared by `lora-tracker pair` with its gateway (listed in `[[peers]]`) and the server. The gateway can decrypt to display or filter, but forwards the original bytes untouched. A gateway with no peers still forwards blind.

```
offset  size  field
0       1     version (1)
1       4     device_id, u32 LE    \
5       4     seq, u32 LE          / header: plaintext, authenticated as AAD
9       n     ChaCha20-Poly1305 ciphertext of postcard(body)
9+n     16    Poly1305 tag
```

- Nonce: `device_id LE || seq LE || 00 00 00 00`. `seq` must never repeat under one key, so firmware keeps a persisted boot counter in the high bits.
- Body fields, in order: `ts` (unix s), `lat_e7`, `lon_e7`, `alt_m`, `speed_cms`, `hdop_x10`, `batt_mv`, `flags`. postcard varints keep a typical packet near 40 bytes. `MAX_PACKET` is 64.
- Flags: bit 0 `NO_FIX` (last known position), bit 1 `STATIONARY`, bit 2 `APPROX` (a city-level estimate from the network's public IP via ip-api.com, not GPS). A board without a GPS fix reports its IP estimate for up to 6 hours, then the last known position with `NO_FIX`.
- The firmware's `seq` is `boot << 16 | n`. The 16-bit boot counter lives in the first sector of the `nvs` partition (`0x9000`, unused by this no_std firmware). After 65536 reports in one boot the board restarts to move it on.
- Receivers call `peek_header` to pick the key, then `open`, and only then `ReplayGuard::accept` (strictly increasing seq per device). The server is the exception: it dedups on `(device_id, seq)` instead, because reports queued on the tracker can arrive out of order over different paths.

Code: `proto/src/packet.rs`.

## Telemetry heartbeat (WiFi)

The same sealed layout and key as a report, with format byte `2` and nonce domain `1` (the last 4 nonce bytes are `01 00 00 00`), so a telemetry `seq` can equal a report `seq` without reusing a nonce. Its `seq` is also `boot << 16 | n`, counted separately. The body is postcard(`Telemetry`): role, board revision, clock, uptime, boot, battery, GPS state and satellites, WiFi RSSI, LoRa tx/rx/rejected counts and RSSI, upload ok/failed/queued, the best position (source GPS or IP, and its age), and up to 6 board log lines (32 chars each) that the server hasn't seen yet. `MAX_TELEMETRY` is 384 bytes. It goes over HTTP only, never LoRa.

A board with a `device_key` sends one as soon as WiFi connects, then every 60 s, position or not. The server accepts only a strictly increasing telemetry `seq` per device (409 otherwise), so a captured heartbeat can't be replayed. The guard resets when the server restarts. The server keeps the latest per device in memory (it isn't stored in SQLite).

Code: `proto/src/telemetry.rs`, built by `fwlogic/src/telemetry.rs`.

## LoRa radio settings

Fixed in `firmware/src/radio.rs`, the same on the V2's SX1276 and the V3's SX1262, so the two boards can talk to each other. Only frequency and TX power come from `[lora]`.

| setting | value |
|---|---|
| spreading factor | 9 |
| bandwidth | 125 kHz |
| coding rate | 4/5 |
| preamble | 8 symbols |
| header / CRC | explicit / on |
| sync word | private (not LoRaWAN) |

A ~40 byte report takes about 250 ms of airtime. A tracker reports at `interval_s` only while GPS says it's moving, and otherwise every `stationary_interval_s`. That includes boards without GPS, since they can't tell.

## HTTP ingest (WiFi, LTE, gateway forwarding)

`POST /ingest` on the URL from `[server]`. The body is one sealed packet, raw bytes, as above (the gateway forwards LoRa packets unchanged). No other auth: the tag proves the sender has the device key.

| status | meaning | board should |
|---|---|---|
| 204 | stored, or already stored (retries are safe) | drop it from the queue |
| 400 | malformed (truncated, wrong version, bad body) | drop it |
| 403 | unknown `device_id`, or the tag didn't verify | drop it |
| 413 | larger than `MAX_PACKET` | drop it |
| 5xx / no answer | server trouble | keep it queued, retry later |

`POST /telemetry` takes one sealed telemetry packet with the same status codes.

Every board request, accepted or rejected (with the reason), is printed to the server's terminal and streamed to `GET /log` (live page) and `GET /api/events` (server-sent events, JSON per line: `at`, `level`, `addr`, `device_id`, `text`). The server keeps the last 300 events in memory for clients that connect later.

Read side (no auth): `GET /` map, `GET /api/devices` (every configured or previously reported device, with `latest` = last report, `last_fix` = last known location, `online` = heard from in the last 150 s, and `seen` = last contact address plus latest telemetry since the server started), `GET /api/devices/{id}/track?hours=24&limit=2000`.

Code: `server/src/routes.rs`.

## Config blob (flash partition)

`b"LTCF"` | version `1` | postcard(`Config`). Trailing erased flash (`0xFF`) is ignored. `MAX_CONFIG` is 1024 bytes.

It lives in the `ltcfg` partition at `0xE000` (4 KiB, data subtype `undefined`, see `firmware/partitions.csv`). `lora-tracker flash-config` writes only that partition, and firmware flashes leave it alone.

Only `device_id` and `role` are required. Each uplink section is optional, and `Config::paths()` decides which paths start:

| path | needs | role |
|---|---|---|
| lora | `[lora]`, plus `device_key` on a tracker | both |
| wifi | `[wifi]` with known networks (or `allow_open` on a tracker), `[server]`, plus `device_key` on a tracker | both |
| lte | `[lte]`, `[server]`, `device_key` | tracker |
| meshtastic | `[meshtastic]` with a PSK | tracker |
| findmy | `[findmy]` with at least one key | tracker |
| ble_status | nothing, always on | both |

A disabled path never stops the others. Hardware that fails its boot probe is handled at runtime by firmware (marked `Unavailable`, retried with backoff).

Code: `proto/src/config.rs`.
