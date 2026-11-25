# Roadmap

1. [x] Scaffold, `proto` packet + crypto + optional-section config, and the `lora-tracker` CLI (setup wizard, pair, edit, flash, flash-config).
2. [ ] Firmware on both boards: GPS + OLED + custom LoRa tracker→gateway, with the gateway printing over serial. Range test. Written for Heltec V2 and V3 (OLED stats / location / last connection / event pages, optional GPS with an IP-location fallback, LoRa TX and RX). Tracker side seen working on a V2; gateway receive and the range test still to do.
3. [ ] Server (axum + SQLite) + gateway WiFi forwarding + tracker direct WiFi on known networks + map page. Server, ingest API and map page done. Firmware WiFi upload, gateway forwarding, SNTP and IP location written, untested on hardware.
4. [ ] BLE GATT status service (position, battery, per-path status).
5. [ ] Find My beacons + server ingest through a self-hosted [macless-haystack](https://github.com/dchristl/macless-haystack).
6. [ ] Meshtastic fallback position packets + server MQTT ingest.
7. [ ] Open WiFi with captive-portal detection (`/generate_204`).
8. [ ] SIM7080G LTE-M (after checking that its TX pin idles at about 3.3V).

Each path is its own task behind an optional config section. Boot probes mark missing hardware `Unavailable` and retry with backoff, and reports queue until any path can deliver them.
