# osx-anytone — macOS Programming Tool for AnyTone AT-D878UVII Plus

A native macOS app to **create, download, and upload codeplugs** for the AnyTone
AT-D878UVII Plus over the USB programming cable — replacing the Windows-only CPS.

> Terminology: AnyTone calls the config a **codeplug** (the user said "CUPs"; the
> real term is codeplug / CPS = Customer Programming Software). This document uses
> "codeplug".

Status: **in progress** · Last updated: 2026-07-12

Progress: Phase 0 ✅ · Phase 1 ✅ (protocol + safe backup/restore, 16 tests) ·
Phase 2 🟢 (model + offline `dump`/`edit` for channels, zones, DMR contacts /
talk groups, RX group lists, and radio IDs — full add / remove / update, with the
contact reverse-lookup tables rebuilt on contact changes; **offsets not yet
validated against real D878UVII hardware**; scan lists / settings still TODO) ·
Phase 3 ✅ code-complete & build-verified (SwiftUI GUI `AnyToneMac` over a C FFI;
`./build.sh` produces `build/Debug/AnyToneMac.app`; Device identify/backup/
restore + offline editor with add/remove/edit panes for every entity family).
**Not yet run end-to-end against real hardware** (see README hardware checkpoints)
· Phase 4 ⏳.

Two remaining tracks: (a) **hardware validation** — needs the physical radio to
confirm the Phase 2 offsets (channels, zones, contacts, group lists, radio IDs,
and the contact index / ID table) match D878UVII firmware; backup/restore is safe
now, the editor is the part awaiting validation; (b) **breadth** — scan lists,
radio settings, and CSV/`.rdt` import-export (Phase 4).

---

## 1. Goal

- Read ("download") the full codeplug from the radio to a file on the Mac.
- Write ("upload") a codeplug from the Mac back to the radio.
- Create / edit codeplug contents (channels, zones, contacts/talkgroups, scan
  lists, radio settings) in a native macOS GUI.
- Never require Windows or the vendor CPS.

## 2. Key research findings (verified from 2+ independent sources)

### USB / transport
- The radio enumerates as a **standard USB CDC-ACM serial device**. On macOS it
  appears as `/dev/cu.usbmodemXXXX` — **no driver or kext required** (macOS ships
  the CDC-ACM driver). This is the single biggest de-risking fact.
- USB IDs seen in the wild: GD32 variant `VID 0x28e9 / PID 0x018a`, STM32 variant
  `VID 0x2e3c / PID 0x5740`. Detect by matching either, then confirm via the
  identify command below (don't hard-fail on VID/PID alone).
- Serial framing: 8-N-1. Baud rate is effectively ignored by the CDC driver but
  set a sane value (e.g. 115200) anyway.

### Programming protocol (byte-level, confirmed by qdmr, dmrconfig, and reald docs)
All multi-byte addresses are **big-endian**.

| Step | Host sends | Radio replies |
|------|-----------|---------------|
| Enter program mode | `"PROGRAM"` (0x50 52 4F 47 52 41 4D) | `"QX"` + `0x06` |
| Identify | `0x02` | `"I"` + model string (e.g. `ID878UV...`) + version/band + `0x06` |
| Read block | `'R'` + addr[4] + len(0x10) | `'W'` + addr[4] + `0x10` + data[16] + checksum[1] + `0x06` |
| Write block | `'W'` + addr[4] + `0x10` + data[16] + checksum[1] + `0x06` | `0x06` |
| Exit program mode | `"END"` | `0x06` |

- **Checksum** = 1-byte sum of (addr[4] + len + data) — i.e. every byte after the
  command byte and before the checksum, mod 256. Excludes the `'R'`/`'W'` byte.
- Block size is 16 bytes for the codeplug data path. (Firmware update uses a
  different 32-byte path with `"UPDATE"` — **out of scope**, do not implement,
  it's the brick-prone path.)
- The codeplug is read/written as a set of memory regions; qdmr's
  `D878UVCodeplug` documents the region list and per-section offsets.

### Prior art to reference (not to bundle)
- **qdmr / libdmrconf** (C++/Qt, GPLv3) — most complete; documents `D878UVCodeplug`
  offsets and the `anytone_interface` protocol. Reference for memory layout.
- **dmrconfig** (C, OpenRTX) — CLI that reads/writes and emits an editable text
  config. Good cross-check for offsets and protocol.
- **reald/anytone-flash-tools** — independent protocol + memory-map writeup.

Links collected in §8.

## 3. Architecture

Follows this repo's existing Swift-GUI-over-Rust-core pattern (see `spell-i/`).

```
osx-anytone/
├── core/                 # Rust: anytone-core (protocol + codeplug model)
│   ├── src/
│   │   ├── transport.rs  # serial trait + macOS CDC-ACM impl + MockTransport
│   │   ├── protocol.rs   # enter/identify/read/write/exit, checksum, framing
│   │   ├── device.rs     # high-level read_codeplug / write_codeplug / backup
│   │   ├── codeplug/     # binary model: channels, zones, contacts, scanlists…
│   │   └── ffi.rs        # C ABI for the Swift app (cbindgen header)
│   └── Cargo.toml
├── cli/                  # Rust: anytone-cli (backup/restore/dump/info)
├── AnyToneMac/           # SwiftUI app (XcodeGen project.yml + build.sh)
├── PLAN.md
└── README.md
```

Rationale:
- **Rust core** matches repo/global conventions, is fully unit-testable without
  hardware (via `MockTransport`), and isolates the byte-twiddling.
- **CLI first** delivers the core user value (download/upload a codeplug file)
  and is the safest way to validate against real hardware before any GUI exists.
- **SwiftUI GUI** via C-FFI is native and reuses the `spell-i` build approach.

## 4. Phased plan

### Phase 0 — Scaffolding & hardware discovery
- Create `core/` (lib) + `cli/` (bin) Cargo workspace; strict clippy.
- Detect candidate serial ports on macOS (`/dev/cu.usbmodem*` + VID/PID enum via
  `serialport` crate). `anytone-cli ports` lists them.
- **Deliverable:** `anytone-cli ports` shows the radio when plugged in.

### Phase 1 — Protocol + full codeplug backup/restore (MVP, highest value)
- Implement `protocol.rs`: enter, identify, read/write block, exit, checksum.
- `MockTransport` that emulates the radio for unit tests (round-trips reads/writes,
  validates checksums, asserts the PROGRAM/END handshake).
- `device.rs`: `read_codeplug()` streams all regions to a `.bin`; `write_codeplug()`
  writes a `.bin` back block-by-block with **read-back verification**.
- CLI: `info` (identify), `backup <file>`, `restore <file>`.
- **Safety gates (mandatory):** refuse `restore` unless a `backup` of the current
  radio state was taken this session or `--force`; verify model string matches the
  file; per-block read-after-write compare; never touch the firmware/UPDATE path.
- **Deliverable:** download a codeplug from the radio and write it back byte-identical.

### Phase 2 — Codeplug binary model (parse → edit)
- Port the region map + section offsets for D878UVII from qdmr's `D878UVCodeplug`.
- Start **read-only**: parse channels, zones, contacts/talkgroups, scan lists,
  radio ID(s); `anytone-cli dump --json`. Validate by parsing a codeplug read from
  the real radio and diffing against what the vendor CPS shows.
- Then **editable**: mutate model → re-serialize → byte-diff vs original to prove
  round-trip fidelity (only intended bytes change).
- Extensive unit tests with golden fixtures (a real backup `.bin`, gitignored if
  it contains personal data; ship a synthetic fixture for CI).
- **Deliverable:** parse + edit + re-serialize channels/zones/contacts losslessly.

### Phase 3 — SwiftUI macOS GUI
- `AnyToneMac` via XcodeGen + `build.sh`; link the Rust staticlib through cbindgen
  C header (mirror `spell-i`).
- Screens: Device (detect/identify/backup/restore with progress + big warnings),
  Channels table editor, Zones, Contacts/Talkgroups, Scan lists, Settings.
- Read from radio → edit → write to radio, all with the Phase-1 safety gates.
- **Deliverable:** end-to-end GUI flow: read, edit a channel, write, verify on radio.

### Phase 4 — Import/export & polish
- CSV import/export for channels & contacts (RadioID/BrandMeister talkgroup lists).
- Best-effort import of vendor `.rdt` codeplug files if the container is tractable.
- README (setup/build/test), error handling hardening, release build + notarization
  notes.

## 5. Risks & mitigations
- **Bricking on write.** Mitigate: mandatory backup-before-write, read-back verify,
  model/firmware guard, hard-exclude the firmware `UPDATE` path, restore is the
  first thing proven to work.
- **Codeplug layout drift across firmware versions.** Capture the identify version
  string; warn when it differs from the layout we target (D878UVII, current fw).
- **No hardware in CI.** All protocol/model logic tested via `MockTransport` +
  golden fixtures; hardware steps are explicit manual checkpoints.
- **Licensing.** qdmr is GPLv3 — reference its documented offsets, don't copy code;
  keep this a clean Rust implementation. reald docs + dmrconfig corroborate offsets.
- **Personal data.** Real codeplug backups may contain callsign/contacts; gitignore
  `*.bin` fixtures, commit only synthetic ones.

## 6. Testing strategy
- `cargo test` in `core/`: checksum vectors, framing, MockTransport round-trips,
  codeplug parse/serialize golden round-trips.
- Manual hardware checkpoints at the end of Phase 1, 2, 3 (documented in README).
- `/verify` before any commit that touches protocol or serialization.

## 7. Open decisions (default chosen, revisit if needed)
- GUI = **SwiftUI** (native, matches repo) vs a Rust TUI/egui. Default SwiftUI.
- Editable scope for v1 = channels, zones, contacts/talkgroups, scan lists, radio
  ID. Deeper menus (roaming, APRS, digital-monitor) deferred to post-v1.

## 8. References
- qdmr: https://github.com/hmatuschek/qdmr · docs https://dm3mat.darc.de/qdmr/
- libdmrconf D878UVCodeplug: https://static.dm3mat.de/qdmr/libdmrconf/classD878UVCodeplug.html
- dmrconfig: https://github.com/OpenRTX/dmrconfig
- reald protocol: https://github.com/reald/anytone-flash-tools/blob/master/at-d878uv_protocol.md
- reald memory map: https://github.com/reald/anytone-flash-tools/blob/master/at-d878uv_memory.md
- ANYTONE serial/factory mode: https://do1alx.de/2022/anytone-factory-settings-mode/
