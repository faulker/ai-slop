# osx-anytone

> # ✅ BOTH WRITE PATHS ARE HARDWARE-VERIFIED
>
> **AnyTone AT-D878UVII Plus: verified.** Confirmed by the operator on
> 2026-09-04, across repeated real writes to a physical radio.
>
> This was the project's longest-standing risk, so it is worth recording what it
> was. The radio erases flash in **32 KB sectors**: any write into a sector
> erases the whole sector. An earlier version rewrote only the blocks it models,
> destroying ~43 KB of everything else (scan lists, tones, roaming, undocumented
> regions) on every write and leaving the **official CPS unable to read the
> radio** (`GetOptionFromCommData_Error`). `write_codeplug` now does **sector
> read-modify-write**: it reads every 32 KB sector it will touch, overlays only
> the modelled blocks, and writes the sector back, preserving everything else.
> That fix is what has now been confirmed in use. See
> [`docs/diagnosing-cps-breakage.md`](docs/diagnosing-cps-breakage.md) for the
> full history, which is still the best guide if a write ever does go wrong.
>
> **QYT KT-5000: verified.** On 2026-08-06 a full read, a one-channel edit, a
> full write, and a post-reboot read-back were run against a real KT-5000. The
> read-back differed from the pre-write backup by exactly the 6 bytes of the
> intended edit, matched the transmitted image byte for byte across the whole
> writable range, left `0x3100`-`0x31FF` untouched, and preserved
> `0x2000`-`0x23EF` exactly.
>
> Since 2026-09-09 that last part no longer depends on the image: `0x2000`-`0x23EF`
> is **excluded from the write map** the way `0x3100`-`0x31FF` already was, so a
> codeplug that was never read off the radio in front of you cannot blank it. One
> did, from the Android app, and erased 950 bytes of calibration-looking data.
> Restoring that range is now an explicit `anytone-cli rescue write --address 0x2000`.
>
> **`backup`, `dump`, `dump-range` and `edit` are read-only and safe.**
>
> Take a backup before writing anyway. The KT-5000 reboots after every transfer,
> so you must reconnect before reading back to verify.
>
> The AnyTone's radio-settings region (`0x02500000`-`0x025014FF`) remains
> **read-only and is never written**, and that has not changed: writing it once
> locked a radio behind an unknown power-on password with Chinese menus.
>
> # ⚠️ USE AT YOUR OWN RISK ⚠️
>
> ## THIS SOFTWARE WRITES DIRECTLY TO YOUR RADIO. IT CAN CORRUPT ITS MEMORY AND LEAVE IT UNUSABLE.
>
> This is an **unofficial, reverse-engineered, AI-generated** tool. It is **not**
> affiliated with, endorsed by, or supported by AnyTone. It has been tested on a
> very limited set of hardware and firmware versions, and the protocol it speaks
> was inferred, not documented.
>
> **Before you connect a radio, understand that:**
>
> - **You can brick your radio.** A bad write, a cable that pops loose mid-restore,
>   or a firmware version this tool has never seen can leave the radio in a state
>   you cannot recover from without vendor tooling, if at all.
> - **You can lose your codeplug.** Restores overwrite everything. Take a full
>   backup with the vendor CPS first, and verify you can read it back.
> - **There is no warranty of any kind.** No guarantee it works, no guarantee it
>   won't destroy your data or your hardware. All risk is yours.
> - **You are responsible for what you transmit.** Nothing here validates that the
>   frequencies, power levels, or modes you program are legal for your license or
>   your region. Check before you key up.
>
> If any of that is unacceptable to you, **stop here and use the official Windows
> CPS instead.**

**Codeplug** is a macOS tool to back up and restore codeplugs for ham radios
over the USB programming cable, without the manufacturer's Windows-only vendor
CPS.

## Supported radios

| Radio | Image size | Auto-detects | Notes |
|---|---|---|---|
| AnyTone AT-D878UVII Plus | 1,789,328 B | yes | DMR + analog. Channels, zones, scan lists, contacts, RX group lists, radio IDs, APRS. |
| QYT KT-5000 | 12,800 B | **no, see below** | Analog only. 200 channels, plus a settings pane for the confirmed header (squelch, beep, scan resume, and a few more) and the LCD display message. Channel editing covers name, RX/TX, both tones, and scan skip. |

The app picks the model for a `.bin` automatically: primarily by file size, which
differs between the two by orders of magnitude, corroborated for the QYT by the ASCII
`Model:` tag the radio stores at `0x0FE0`. Opening a codeplug switches the sidebar to
that radio's sections, so a QYT document shows channels, settings, and VFO channels, while an
AnyTone document shows the full DMR set.

Writing is guarded in both directions: the model is detected from the **file**, and the
core refuses to write unless the connected radio's identity matches, before any block
is transmitted.

### The QYT KT-5000 does not auto-detect yet

Its USB VID/PID does not appear anywhere in the captures this support was built from,
so an unrecognized adapter is not assumed to be a KT-5000. **Select the model
explicitly in the Device pane** for any port whose USB identity is unknown, then read
or write as normal. Everything works; only auto-detection is missing.

If you want to fix that, run `system_profiler SPUSBDataType` with the radio plugged in
and add the VID/PID to `KNOWN_USB_IDS` in `core/src/ports.rs`, where a TODO marks the
spot. A guessed ID is worse than none, since it would silently match unrelated
USB-serial adapters.

Protocol and memory layout for the KT-5000 are documented in
[`docs/qyt-kt5000-memory-map.md`](docs/qyt-kt5000-memory-map.md). Two behaviours worth
knowing before you connect it: **the radio reboots after every read or write**, so
verification is always a separate session, and its settings block is only partly
modelled on purpose (the reference driver everyone else uses has a known-wrong map for
this model, so unlabelled bytes are preserved rather than guessed). Beep and scan
resume are in the pane because a CPS one-field-diff pass confirmed them; scan resume
even sits one byte earlier than that reference map claimed.

The pane's **Display message** field is the line the radio shows on its own screen,
the 16 ASCII bytes at `0x0FE0`. A stock radio reads `Model:KT-5000` there because
that is what the factory writes into it, not because it is fixed; type over it to put
a callsign or anything else up to 13 printable ASCII characters on the display. The
change is staged like any other edit, so it reaches the radio on the next write.

The same caution applies to a channel's packed flag bytes. **Scan skip is editable** —
its bit was confirmed against a real radio, and the channel editor's Scan section writes
it — while power, bandwidth, BCL, PTT-ID and the rest are shown under a collapsed
**Radio flags (read-only)** row and round-trip untouched until their bit assignments are
confirmed the same way. The channel table's **Scan** column marks the skipped channels
so they can be spotted without opening each one. Multi-select (⌘-click / shift-click)
also opens **Bulk Edit**, which can set scan skip across the selection in one step.

See `PLAN.md` for the full design and phased roadmap. This repo implements
**Phase 0 (scaffolding + port discovery)**, **Phase 1 (protocol + full codeplug
backup/restore)**, a **Phase 2 codeplug model** covering channels, zones, scan
lists, DMR contacts / talk groups, RX group lists, and radio IDs (full add /
remove / update), and the **Phase 3 SwiftUI GUI** (`AnyToneMac/`, app name
Codeplug). The firmware `UPDATE` path is deliberately excluded because it can
brick the radio.

### CPS-parity settings (added for closer feature parity with the vendor CPS)

- **Channels** now model ~30 fields: analog CTCSS/DCS encode+decode, squelch
  mode, optional signaling, DTMF/2-Tone/5-Tone IDs, TX-permit (admit), scan-list
  assignment, and the flags (PTT-prohibit, talk-around, work-alone, call-confirm,
  APRS-RX, simplex-TDMA). All read+write, offsets from qdmr.
- **Scan lists** are a full read+write tab: members, priority channels 1/2,
  revert channel, look-back A/B, dropout delay, dwell time.
- **APRS** and **Zone A/B channel** are shown **read-only**. They live in a
  protected radio-settings region that must never be written (writing it once
  locked a radio behind an unknown power-on password with Chinese menus), so the
  tool displays them for reference but does not write them back. See
  [`docs/codeplug-memory-map.md`](docs/codeplug-memory-map.md).

> **⚠️ Codeplug size changed.** Adding the scan-list and APRS regions to the
> read path grew the backup size, so **`.bin` backups made before this change no
> longer parse**. Read a fresh backup from the radio after upgrading.

## Layout

- `core/` — `anytone-core` library: byte transport (real + mock), the
  byte-level programming protocol, high-level backup/restore, port enumeration,
  the codeplug model (channels, zones, scan lists, DMR contacts/talk groups, RX
  group lists, radio IDs, plus read-only APRS + zone A/B) with lossless
  parse → edit → re-serialize and JSON batch editing
  (`src/anytone/codeplug/edits.rs`), and two host-facing surfaces over the same
  layers: the C FFI for the macOS app (`src/ffi.rs`, header in
  `core/include/anytone_core.h`) and the UniFFI surface for the Android app
  (`src/api.rs`).

  Both are behind features. `desktop` (the default) brings in `serialport` and
  with it `SerialTransport`, port enumeration, and the C ABI. `uniffi-api`
  brings in `src/api.rs`, where the transport is supplied by the host language
  instead, because `serialport` needs libudev and Android does not have it.
  The USB identity table lives in `src/usb_ids.rs`, outside both, so the two
  platforms attribute radios from one table.
- `cli/` — `anytone-cli` binary: `ports`, `info`, `backup`, `restore`, `dump-range`, `rescue`, `dump`,
  `edit`.
- `AnyToneMac/` — SwiftUI macOS app (Device backup/restore + offline codeplug
  editor with add/remove/edit for every entity family), linked against the
  `anytone-core` staticlib. Project is generated by XcodeGen from `project.yml`;
  built by `build.sh`.
- `AnyToneMacTests/` — Swift tests for the app layer (formatting, edit-spec
  encoding, and the save/recovery model).
- `android/` — Jetpack Compose Android app over the same core, reaching the
  radio through the USB host API instead of a serial port. Has its own
  [`README.md`](android/README.md). Built by `android/build.sh`.
- `scripts/gen-bindings.sh` — regenerates the Android app's Kotlin bindings from
  `core/src/api.rs`. The macOS app is not generated; it keeps the hand-written
  C header.
- `tools/make-icon.swift` — draws the app icon with CoreGraphics and writes the
  asset catalog's PNGs. Re-run after changing it.
- `docs/` — [`codeplug-memory-map.md`](docs/codeplug-memory-map.md) (region
  addresses and layouts, including the regions that must never be written) and
  [`diagnosing-cps-breakage.md`](docs/diagnosing-cps-breakage.md) (playbook for
  when the vendor CPS can no longer read a radio this tool has written).

## Setup

Requires a Rust toolchain (`rustup`, stable). No macOS driver/kext is needed; the
AnyTone enumerates as a standard CDC-ACM serial device (`/dev/cu.usbmodem*`) and
the KT-5000's cable as a Prolific PL2303 (`/dev/cu.usbserial*`).

For the macOS GUI you also need Xcode (with `xcodebuild`) and
[XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).

For the Android app you need the Android SDK and NDK, `cargo-ndk`, and the
Android Rust targets. See [`android/README.md`](android/README.md).

## Build & test

```sh
# Rust core + CLI
cargo build
cargo test
cargo clippy --all-targets

# macOS app (builds the Rust staticlib first, then the app; ad-hoc signed)
./build.sh
# → build/Debug/AnyToneMac.app

# Swift app tests
xcodebuild test -project AnyToneMac.xcodeproj -scheme AnyToneMac \
  -destination 'platform=macOS'

# App icon (regenerate after editing the drawing code)
swift tools/make-icon.swift

# Android app. Finds its own JDK and SDK; cross-compiles the core into
# jniLibs, then builds the APK.
android/build.sh
# → android/app/build/outputs/apk/debug/app-debug.apk

android/build.sh test       # host JVM tests, no device or radio needed
android/build.sh install    # build and adb install
android/build.sh release    # release APK, unsigned
android/build.sh bindings   # regenerate the Kotlin bindings from core/src/api.rs
android/build.sh clean
```

### Release build

`./build.sh release` produces a distributable app: a universal (arm64 + x86_64)
Release build, signed with a Developer ID certificate and the hardened runtime,
packaged as a DMG in `dist/`.

```sh
./build.sh release
# → dist/AnyToneMac.app and dist/AnyToneMac-<version>.dmg
```

It picks the first "Developer ID Application" certificate in your keychain, or
set `SIGNING_IDENTITY` to choose one. With no such certificate it falls back to
an ad-hoc signature, which runs on your own machine but is blocked by Gatekeeper
everywhere else.

For a DMG that opens cleanly on someone else's Mac you also need notarization.
Store an app-specific password once, then pass the profile name:

```sh
xcrun notarytool store-credentials anytone-notary \
  --apple-id you@example.com --team-id TEAMID --password <app-specific-password>

NOTARY_PROFILE=anytone-notary ./build.sh release
```

The DMG and app are then submitted to Apple and stapled, so they work offline.
Bump `MARKETING_VERSION` in `project.yml` to change the version in the DMG name.

All protocol and device logic is unit-tested against an in-memory `MockTransport`
that emulates the radio, so the full suite runs with no hardware attached. The
Swift tests likewise need no radio: they drive the real core against a synthetic
all-zero codeplug image, covering number formatting, the `EditSpec` JSON contract,
and the save/recovery model.

## Usage

```sh
# List serial ports; likely radios are marked "* radio".
anytone-cli ports

# Identify the connected radio (auto-selects the port if exactly one radio).
anytone-cli info

# Download the full codeplug to a file.
anytone-cli backup my-codeplug.bin

# Upload a codeplug back to the radio (requires --force; verifies each block).
anytone-cli restore my-codeplug.bin --force

# Inspect a codeplug offline as JSON (channels, zones, contacts, group lists,
# radio IDs).
anytone-cli dump my-codeplug.bin

# Apply a JSON edit batch offline (same schema as the GUI / anytone_apply_edits).
anytone-cli edit my-codeplug.bin edits.json -o edited.bin

# Read an arbitrary address range off the radio (read-only diagnostic).
# Hexdump the radio-settings block, which backups do not cover:
anytone-cli dump-range --address 0x02500000 --length 0x1500

# ...or save it raw for diffing.
anytone-cli dump-range --address 0x02500000 --length 0x1500 -o settings.bin
```

Use `--port /dev/cu.usbmodemXXXX` to select a specific port when auto-detect is
ambiguous.

`dump-range` never writes; it is there to inspect regions outside the codeplug
region map. `--address` and `--length` take decimal or `0x`-prefixed hex, and
the length must be a multiple of the 16-byte block size.

### Rescuing a KT-5000 that will not stay up

`restore` needs about 18 seconds of link at this radio's 9600 baud, and a radio
in a reset loop gives you less than that, so `restore` can never finish. The
`rescue` family retries the handshake until the radio answers, transfers until
the link dies, then knocks again and **picks up where it stopped**. A full write
of a boot-looping KT-5000 took 26 windows and landed every block.

```sh
# Listen only. Sends nothing; prints a timestamped hexdump of whatever the
# radio says. A boot loop shows up as its NXP boot banner repeating, and the
# gap between repeats is the loop period.
anytone-cli rescue watch --port /dev/cu.usbserial-10 --seconds 30

# Knock until the radio answers, then measure how long the link keeps
# answering. Read-only. This is the number that decides what is possible:
# under ~20s means no full restore, ever.
anytone-cli rescue knock --port /dev/cu.usbserial-10

# Read the whole image back and say how it differs from what you sent,
# region by region. Read-only, and the fastest way to find out whether the
# codeplug is still a suspect at all.
anytone-cli rescue verify kt5000-backup.bin

# Read one range (read blocks are 0x40; the range may go anywhere in the
# 16-bit space, not just the 12,800 bytes a backup covers).
# The settings header and the VFO records, the two most likely places for a
# byte that upsets the firmware:
anytone-cli rescue read --address 0x0E00 --length 0x200 -o suspect.bin

# Write a range back out of a known-good backup (write blocks are 0x10; the
# range must end at or below 0x3100). Use --address 0x0000 --length 0x3100 for
# a full restore that survives the resets.
anytone-cli rescue write kt5000-backup.bin --address 0x0E00 --length 0x200 --force
```

**Always `verify` after a `rescue write`.** On a radio that resets mid-transfer
the acknowledgement is not proof: the radio acks a write on receipt and commits
it afterwards, so a reset in between silently loses a block. The cursor re-sends
the last block of any window that ended badly, which catches most of it, and the
read-back catches the rest.

A rescue **read** may go anywhere in the 16-bit address space, not just the
12,800 bytes a backup covers: the session prologue reads `0x3DF0` and gets an
answer, so there is more up there than the vendor CPS transfers, and looking
costs nothing.

`watch`, `knock`, `read` and `verify` never write. `rescue write` does, and it takes the
bytes from a full 12,800-byte image so the address in the file and the address
in the radio are the same number.

The `edit` batch object has, for each entity family, an update array
(`channels`/`zones`/`contacts`/`group_lists`/`radio_ids`, each element an index
plus optional fields), an `add_*` array (field objects without an index), and a
`remove_*` array of indices; operations run update → remove → add. Example that
adds a talk group and a digital channel wired to it:

```json
{
  "add_contacts": [{"name": "LOCAL 2", "number": 2, "call_type": "group"}],
  "add_channels": [{"name": "TG2 TS1", "rx_frequency_hz": 439000000,
                    "mode": "digital", "color_code": 1, "time_slot": 1,
                    "contact_index": 0}]
}
```

## GUI (Codeplug)

A sidebar selects between the radio and the open codeplug's entity families.
Both are thin shells over the Rust core.

- **Device** — lists the connected AnyTone radios (matched by USB VID/PID; macOS
  enumerates every serial device twice, as `/dev/cu.*` and `/dev/tty.*`, and the
  core collapses each pair to its callout half so one radio shows up once). With
  none connected the list is replaced by a Refresh button. Identify shows the
  model string, **Read from Radio…** reads the full codeplug to a `.bin`, and
  **Write to Radio…** writes a `.bin` back after a prominent confirmation
  dialog. A read or write shows a centered progress overlay while it runs (it can
  take minutes) and a completion dialog when it finishes; the read's dialog offers
  to open the file it just produced straight in the editor. The same safety gates
  as the CLI apply (model check, per-block read-back verification).
- **Codeplug** — opens a `.bin` offline (no radio needed) and lists Channels,
  Zones, Contacts, Group Lists, and Radio IDs. The app starts with **no codeplug
  open**; the toolbar's **Open** (folder) button is always available, and a
  **Close** button appears once a file is open — neither needs the File menu.
  Opening a file whose size doesn't match this build's radio is refused up front
  with a plain-language message, so a codeplug from a different model or an older,
  incompatible firmware version can't be half-loaded. Every table sorts by any column
  and filters from the toolbar's search field; the Channels tab adds a filter bar
  to narrow by mode, color code, and time slot (with a Clear button once any is
  set). Double-click a row (or press
  Edit) to open its editor in a centered sheet: Escape or Cancel closes it
  without keeping anything, Done stages the change. Adding an entry opens that
  same editor immediately, so a new record is never left as a blank "NEW" row;
  a new channel starts with its RX group list set to "None". Channels expose
  name, RX/TX, mode, power, bandwidth, color code, time slot, and the DMR contact /
  group-list / radio-ID selection (name pickers, not raw indices). The digital
  fields (color code, time slot, contact, RX group list, radio ID) are disabled
  for pure Analog channels, since they carry no DMR data. Contacts
  expose name, DMR ID, and call type (a talk group is a Group call); radio IDs
  edit name and DMR ID. Every change round-trips through the Rust `Codeplug`
  model, which re-verifies the result by re-parsing. Removing a channel scrubs
  it from zones, and removing a contact scrubs it from group lists.
- **Building zones and group lists in bulk.** A zone's channels and a group
  list's contacts are edited as a two-column transfer: everything available on
  the left, everything already a member on the right, and buttons between them
  that move a whole multi-selection at once (⌘-click / shift-click to pick
  several). Double-clicking a single row moves just that one across — the fast
  path for adding or removing one member. Both columns filter, which is what
  makes the left one usable when a codeplug carries thousands of channels or
  contacts. Membership is still chosen by name; the slot indices stay hidden.
- **Writing from the Codeplug tab.** The Codeplug toolbar has its own **Write to
  Radio** dropdown, so you don't have to save and switch tabs to try an edit on the
  radio. The menu lists every connected radio (with a Refresh entry to re-scan);
  picking one writes what you are looking at, staged changes included, and says so
  in the confirmation.
- **Undo and redo.** ⌘Z reverses the last staged change and ⇧⌘Z reapplies it,
  across every kind of edit — field changes, Add, Remove, Move, and bulk edits.
  History is per document and resets when you open another one.
- **Seeing what's unsaved.** A record with staged changes gets an orange dot in
  its row, and any section holding staged changes gets one next to it in the
  sidebar, so you can find your edits without remembering where you made them.
- **Choosing where an entry lands.** The `#` column is a 1-based slot number. A
  radio's channels are sparse (real codeplugs skip slots, so numbers run
  `1, 2, 3, 4, 10, 11, …`). "Move to Slot…" in a row's context menu **moves**
  that entry to a chosen free slot and fixes up everything that referenced it (a
  moved channel is remapped in every zone; a moved contact in every channel and
  group list). This is also how you place a newly added entry: Add it, then move
  it. Moving onto an occupied or out-of-range slot is rejected. Because members
  are chosen by name, zone and group-list references always line up with what
  the tables show.

### Nothing is written until you save

Your `.bin` is never modified until you press **Save** (⌘S) — not by editing a
record, not by Add, Remove, or Move. Staged changes live in a work file under
`~/Library/Application Support/AnyToneMac/Recovery/`, and the window subtitle
reads "Edited" whenever the file on disk is behind what you see.

**Save As…** (⇧⌘S, or the chevron next to the toolbar's Save button) writes the
staged state to a different `.bin` and continues editing that one, leaving the
original as it was. It works on an unmodified codeplug too, which is the easy way
to fork a working setup before experimenting on it.

That work file doubles as crash protection: if the app dies (or is force-quit)
with unsaved changes, the next launch offers to restore them. Quitting, closing,
or opening another codeplug with unsaved work prompts first. Closing (or a clean
launch with nothing to recover) clears the work file, so no stale staging buffer
is ever left behind.

This matters for a codeplug specifically — it can represent hours of work and is
often the only copy. `SaveModelTests` enforces the guarantee by hashing the file
before and after each operation.

### Manual hardware checkpoints (need the physical radio)

- `anytone-cli info` / GUI Identify shows the D878UVII model string.
- GUI Read from Radio produces a `.bin` byte-identical to `anytone-cli backup`.
- GUI Write to Radio of an unmodified read completes with read-back verification.
- Edit one channel name in the Codeplug tab, write it, and confirm on the radio.
- With one radio plugged in, the Device list shows exactly one row (not the
  `cu.`/`tty.` pair).

## Safety

- `restore` refuses to write unless `--force` is passed, and prints a loud warning.
- The identify model string is checked before any write (must look like a
  D878UV-family radio).
- Every written block is read back and compared; a mismatch aborts the restore.
- Always take a `backup` before your first `restore`.

### How the transfer works, and which details are load-bearing

Backup/restore transfers the codeplug the way the vendor CPS does. **Reads** pull
the channels/zones in full plus the DMR bitmaps and only the active DMR records.
**Writes are fully sparse** — every valid bitmap (small) followed by only the
*active* channels, zones, contacts, group lists, and radio IDs, then the contact
index / ID table sized to the contact count, all sent in **ascending address
order**. Several details are load-bearing and were found by testing on real
hardware:

- the write command must end with a trailing `0x06` byte, or the radio never ACKs;
- flooding the radio with writes to every unused slot (all 32 channel banks, etc.)
  resets it mid-transfer, so only active records are written;
- records must precede the bitmap that references them (address order), because
  the radio commits the whole codeplug on program-mode **exit**, not per block;
- for that same reason writes are **not read-back verified** — an immediate read
  returns the pre-commit value, so verification would spuriously fail on every
  real edit. Each block's write ACK is the transfer check (exactly as the vendor
  CPS / qdmr do); confirm an edit by re-reading after the radio reboots.

Validated end-to-end on a real **AT-D878UVII Plus (firmware V101)**: backup →
restore round-trips byte-for-byte, and an edit (add a channel) written back and
re-read shows the change landed correctly. Note that codeplug `.bin` files
captured before the DMR regions, or before the zone channel list (0x02500100),
were added will no longer parse — re-read from the radio. Because writes
are not read-back verified, keep a known-good backup and, before trusting an
*edited* codeplug written to the radio:

1. `anytone-cli backup real.bin` from your radio.
2. `anytone-cli dump real.bin` and confirm the channels/zones/frequencies match
   what the vendor CPS shows for the same codeplug.
3. Only then edit + restore, starting with a single harmless change (e.g. one
   channel name) and confirming it on the radio.

If the dump does not match the CPS, the offsets need adjusting for your firmware
before the editor is safe to use — file that as the next task.
