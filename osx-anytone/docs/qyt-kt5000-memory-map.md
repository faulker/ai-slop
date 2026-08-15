# QYT KT-5000 protocol and memory map

Reference for the second radio this app programs. The AnyTone equivalent is
`codeplug-memory-map.md`; this document mirrors its structure.

Everything here is derived from two sources:

1. **USB captures of the vendor Windows CPS** talking to a real KT-5000, kept in
   `docs/reference/QYT/` (gitignored). Eight raw byte streams covering a full read,
   a full write, a config-only read, and a config-only write, each split into host
   and radio directions.
2. **CHIRP's `chirp/drivers/btech.py`**, read as a hardware-facts reference for the
   field layouts our captures could not resolve, then reimplemented independently
   in Rust. CHIRP is GPLv3 and this project is MIT, so no code was copied; this is
   the same treatment `PLAN.md` already documents for qdmr. Working notes with the
   verbatim struct quotes are in `docs/reference/qyt-kt5000-chirp-notes.md`.
   Sources: <https://raw.githubusercontent.com/kk7ds/chirp/master/chirp/drivers/btech.py>
   and <https://github.com/kk7ds/chirp/pull/1302>.

Claims below are marked **confirmed** when they hold in our own capture bytes, and
**CHIRP only** when they come from the driver without a capture example.

---

## Wire protocol

A different protocol family from the AnyTone: 16-bit addresses, no checksums,
different command bytes, and asymmetric read/write block sizes.

| Step | Bytes |
|---|---|
| Host handshake | `55 20 21 01 23 FF DC 02` |
| Radio identity reply | 61 bytes: `A1 20 00 00 00 00 02 00 02 00` `06` + 10-byte header + `"KT-500"` + same header + `"WP9114"` + `04 00 00 04 08 00 00 01` twice + `55 05` |
| Host session prologue | `06 53 3D F0 10` |
| Prologue reply | `58 3D F0 10` + 16 bytes, **no leading `06`** |
| Host read | `06 53 AH AL LL` where `LL` = `0x40` |
| Read reply | `06 58 AH AL LL` + `LL` bytes, **with leading `06`** |
| Host write | `06 58 AH AL LL` + `LL` bytes, `LL` = `0x10` |
| Write reply | bare `06` |
| End of a read session | host sends one bare `06` |
| On port close | radio emits `A1 20 00 00 00 00 02 00 00 00` `06` + a 106-byte NXP UART demo banner, then reboots |

`0x53` is ASCII `'S'` (read), `0x58` is `'X'` (write). Addresses are **16-bit
big-endian**. There is **no checksum anywhere** in either direction. All confirmed.

### The asymmetric leading `0x06`

The prologue reply omits the leading `0x06`; every data read reply carries it. Our
`read_block` therefore tolerates an optional leading `0x06` before the `0x58`. That
single tolerance also absorbs stray chatter, which is why no resync machinery is
needed. Confirmed against all four capture pairs.

### The `0x3DF0` prologue

Every operation, read or write, begins by reading 16 bytes at `0x3DF0`. The radio
returns 12 ASCII spaces followed by four zero bytes. Its purpose is unknown; a
firmware/region probe and a link-alive check are both plausible. We reproduce it
because it costs one 16-byte read and omitting an observed handshake step is the
riskier choice.

### Stream arithmetic

These reconcile exactly, which is what makes byte-identical golden tests possible
without hardware (`core/tests/qyt_capture.rs`):

| Capture | Bytes | Breakdown |
|---|---|---|
| `read-radio_write` | 1019 | 8 + 5 + 201x5 + 1 |
| `read-radio_read` | 14067 | 61 + 20 + 201x69 + 117 |
| `write-radio_write` | 16477 | 8 + 5 + 784x21 |
| `write-radio_read` | 982 | 61 + 20 + 784x1 + 117 |

Note **201** reads, not 200: the vendor CPS reads address `0x0000` **twice** before
walking `0x0000` to `0x31C0` in `0x40` steps. We reproduce the duplicate read so our
host stream matches the vendor's byte for byte. Writes cover `0x0000` to `0x30F0` in
`0x10` steps, 784 blocks.

### Reboot behaviour

**The radio reboots after every read or write, and there is no exit command.** So
there is no in-session read-back verification of a write; the vendor CPS does not
attempt one either. Verification is always a separate post-reboot session: close the
port, wait, reconnect, read. This is the main operational difference from the
AnyTone path, whose `write_region` verifies each block inline.

---

## Memory map

Image is `0x0000`-`0x31FF`, 12800 bytes. An unprogrammed channel slot is all `0xFF`.

| Range | Contents | Status |
|---|---|---|
| `0x0000`-`0x0C7F` | Channel records, 200 x 16 B | confirmed |
| `0x0C80`-`0x0DFF` | `0xFF` | confirmed |
| `0x0E00`-`0x0E35` | Settings header (modelled) | confirmed |
| `0x0E36`-`0x0EFF` | Further settings, **not modelled**, byte-preserved | see below |
| `0x0F00`-`0x0F7F` | Four 32-byte VFO-shaped slots; only A and B are real VFOs | confirmed |
| `0x0FE0`-`0x0FEF` | ASCII `Model:KT-5000   ` | confirmed |
| `0x1000`-`0x1C7F` | Channel names, 200 x 16 B, index-parallel | confirmed |
| `0x2000`-`0x23EF` | Real writable memory; holds whatever was last written. Preserve it. | confirmed |
| `0x2AF0`-`0x2AFF` | Factory blob | confirmed |
| `0x3000`-`0x30FF` | DTMF / autodial memories, 16 records | hypothesis |
| `0x3100`-`0x31FF` | `0xFF`; read but **never written** | confirmed |
| `0x3DF0`-`0x3DFF` | Prologue probe target | confirmed |

Channel capacity of 200 is confirmed two ways: CHIRP declares `_upper = 199`, and our
capture holds 159 programmed plus 41 empty slots, summing to exactly 200. The map is
also self-consistent: 200 x 16 = `0x0C80` fits under the settings block at `0x0E00`,
and `0x1000` + `0x0C80` = `0x1C80` fits under `0x2000`.

---

## Channel record

16 bytes at `0x0000 + index * 16`, with the name at `0x1000 + index * 16`.

| Off | Field | Encoding |
|---|---|---|
| 0-3 | RX frequency | packed BCD **little-endian**, 10 Hz units |
| 4-7 | TX frequency | same; `FF FF FF FF` means TX not set |
| 8-9 | RX tone | `u16` little-endian, see below |
| 10-11 | TX tone | `u16` little-endian |
| 12 | `unknown0:4, scode:4` | low nibble is the PTT-ID signal code index |
| 13 | `unknown1:2, spmute:2, unknown2:2, optsig:2` | |
| 14 | `unknown3:3, scramble:1, unknown4:2, power:2` | |
| 15 | `unknown5:1, wide:1, unknown6:2, bcl:1, add:1, pttid:2` | |

**Bit sub-fields are listed MSB first**, so the first-named sub-field occupies the
high bits. In byte 15 that puts `wide` at bit 6, `bcl` at bit 3, `add` at bit 2, and
`pttid` in bits 1-0.

The frequency encoding differs from the AnyTone, which uses BCD **big-endian**. Worked
example, channel 0: `50 62 25 46` reversed is `46 25 62 50`, read as decimal digits
46256250, times 10 Hz = **462.56250 MHz**, which is FRS/GMRS channel 1. Confirmed.

### Flag bits confirmed on hardware, 2026-08-06

The original captures could not resolve these: every programmed channel carried
identical flag bytes, so there was nothing to diff. The owner then changed one field at
a time in the vendor software, and a fresh read was diffed against the image we had just
written. Thirteen bytes changed, all exactly where CHIRP said they would be.

| Field changed | Slots | Byte | Transition | Conclusion |
|---|---|---|---|---|
| Power to Low | 0-2, 163-165 | 14 | `0x00` -> `0x01` | **power = byte 14 bits 1-0**, `0` = High, `1` = Low |
| Scan to skip | 3-5 | 15 | `0x04` -> `0x00` | **add = byte 15 bit 2**, set means in scan, clear means skipped |
| BCL on | 6-8 | 15 | `0x04` -> `0x0C` | **bcl = byte 15 bit 3** |

Pinned by `flag_bits_match_the_hardware_diff` in `core/src/qyt/codeplug/channel.rs`.

**`add` is the one flag bit with a setter.** `QytChannel::set_scan_add` rewrites bit 2 of
byte 15 and nothing else, and the edit spec exposes it as `scan_add` on a channel update
or add. Power, BCL and the rest stay read-only: confirming that a bit *decodes* correctly
is not the same as confirming the radio accepts a write there, and scan-skip is the only
one anyone has asked to change.

**`unknown6` (byte 15 bits 5-4) looks vestigial.** Slot 0 was the only channel that ever
carried `0x24` rather than `0x04`. Editing its power in the vendor software cleared those
two bits to `0x04`, so the vendor tool does not preserve them and the radio did not mind
losing them. We still round-trip them untouched: nothing is gained by clearing bits we do
not understand.

**Bandwidth is still unconfirmed.** No channel had its bandwidth changed in this pass, so
`wide` (byte 15 bit 6) remains CHIRP-derived. Every channel reads `0`, which
`MODES = ["FM", "NFM"]` makes wide, and that matches a GMRS plus 2 m / 70 cm mix.

Value tables (**CHIRP only** except the three confirmed above, and consistent with every
captured value):

| Field | Values |
|---|---|
| `power` | 2 bits, only 2 used: `0` = High (25 W), `1` = Low (5 W) |
| `wide` | `MODES = ["FM", "NFM"]`, so **`0` = wide, `1` = narrow** |
| `add` | `0` = skipped, `1` = in scan |
| `bcl`, `scramble` | booleans |
| `pttid` | `OFF`, `BOT`, `EOT`, `BOTH` |
| `optsig` | `OFF`, `DTMF`, `2TONE`, `5TONE` |
| `spmute` | `Tone/DTCS`, `Tone/DTCS and Optsig`, `Tone/DTCS or Optsig` |
| `scode` | index into codes 1-15; raw `15` is treated as `0` |

Note `wide` = 0 meaning wide is easy to misread as "everything is narrow". All 159
programmed channels in the capture read 0, which is correct for a mix of GMRS and 2 m
/ 70 cm repeaters.

### Byte 15 bits 5-4 are unnamed

CHIRP calls these `unknown6` and does not name them in **any** BTECH format in the
file. They are the entire difference between channel 0 (`0x24`) and every other
programmed channel (`0x04`). We round-trip them opaquely and never synthesize a
value. `unknown6_bits_survive_a_roundtrip` asserts this.

### Names

16 bytes, maximum 7 ASCII characters, `0xFF`-padded. An empty slot's name record is
all `0xFF`. Confirmed: 157 of 159 records have `0xFF` from offset 7 onward; two carry
a stray `0x05` at offset 7, which is leftover garbage rather than a field.

---

## Tone encoding

One scheme covers CTCSS and DCS in the same `u16`:

| Raw value | Meaning |
|---|---|
| `0x0000` | no tone |
| > 600 | CTCSS, frequency in 0.1 Hz units |
| 1-105 | DCS normal |
| 106-600 | DCS inverted (**CHIRP only**, no capture example) |

DCS values index `sorted(DTCS_CODES + [645])`.

Worked examples, all confirmed: `0x0375` = 885 = **88.5 Hz** CTCSS; `0x03E8` = 1000 =
**100.0 Hz**. The six distinct small values in the capture (1, 3, 15, 16, 41, 88)
decode to **D023N, D026N, D073N, D074N, D245N, D606N**, all real DCS codes, which is
strong confirmation of the index scheme. Split tone is supported and used: channel 64
(`N7FYZ`) has no RX tone and a 100.0 Hz TX tone, the classic repeater configuration.

The inverted branch is the same arithmetic on the other side of the 105 boundary, but
nothing in the capture exercises it, so treat it as unverified until hardware says
otherwise.

---

## VFO channels

The image holds four 32-byte slots at `0x0F00` (A–D), inherited from the WP-9900
layout. **The KT-5000 only has two VFOs (A and B).** Slots C and D are parsed and
written back verbatim so they survive a round trip, but they are not exposed in
the dump, the UI, or the edit path.

| Off | Field |
|---|---|
| 0-7 | frequency digit string |
| 8-13 | offset |
| 16 | RX tone |
| 18 | TX tone |
| 20 | scode |
| 21 | spmute |
| 22 | optsig |
| 23 | scramble |
| 24 | wide |
| 25 | power |
| 26 | shift direction |
| 27 | step |

### The encoding divergence, read this before touching VFO frequencies

**`freq[8]` does NOT use the packed BCD of the channel table. It stores one decimal
digit per raw byte.** Confirmed by decoding record 0 to 462.5625 MHz, which matches
channel 0 exactly.

This is the sharpest trap in the format. Reading these bytes as packed BCD does not
fail loudly; it produces a plausible-looking wrong frequency. The two decoders are
kept deliberately separate in `core/src/qyt/codeplug/bands.rs` for that reason, and
`band_frequency_uses_one_digit_per_byte` pins it.

---

## Write policy

**`0x3100`-`0x31FF` is read but never written.** It reads back as `0xFF` and the
vendor CPS never writes it. Corroborated independently: CHIRP's `UPLOAD_MEM_SIZE` is
`0x3100`, and 784 writes x `0x10` lands exactly there. We read it into the image so
backups stay faithful and diffable, and exclude it from the write map.
`qyt_write_never_touches_0x3100_or_above` asserts this.

**`0x2000`-`0x23EF` is real, persistent memory. Preserve whatever is there.**

An earlier version of this document claimed the radio read that region back as `0xFF`
regardless of what was written, and concluded the vendor's bytes were a harmless
uninitialized buffer. **That was wrong**, and the mistake is worth recording because it
is easy to repeat: the `read-radio` capture was taken *before* the `write-radio`
capture. The read showed `0xFF` because the write had not happened yet, not because the
region rejects writes.

Confirmed on hardware 2026-08-06: a fresh read of the same radio returns those 950
bytes **byte-for-byte as the CPS wrote them**. Some of it looks structured rather than
random (for example an ascending `07 04 04 04 05 05 06 06` run at `0x2100`), so do not
assume it is junk.

The practical rule: the write path sends exactly what the supplied image holds, so a
read-modify-write round trip preserves this region automatically. Only an image
synthesized from scratch would blank it, and nothing in the app does that. A golden
test pins that our write stream reproduces the image it was given.

**General lesson for this codeplug.** Two captures taken at different times are not two
views of one state. Diffing them shows change over time, not a property of the radio.
Any conclusion of the form "the radio ignores writes to X" needs a write followed by a
read in that order, on the same radio, with a reboot in between.

---

## Settings

**`0x0E00`-`0x0E35` is modelled**: `tmr`, `squelch`, `tot`, `volume`,
`backlight_timer`, `beep`, `scan_rev`, and `remember_power`. Unnamed bytes inside
that range live in `raw` and are never rewritten.

`tmr` is Menu 0 TMR (dual-band waiting): **Off / On only**, matching the KT-5000
manual. CHIRP's WP-9900 `LIST_TMR7` (`M+A`…`M+ABC`) does not apply; this radio
has no A/B/C watch areas and no scan lists.

### Settings confirmed on hardware, 2026-08-06 / 2026-08-14

One-field-at-a-time diffs. Each variant differs from its baseline by exactly one
byte:

| Field changed | Address | Transition | Conclusion |
|---|---|---|---|
| Squelch 6 → 3 | `0x0E02` | `0x06` → `0x03` | **sql already modelled**; confirms the offset |
| Beep off → on | `0x0E09` | `0x00` → `0x01` | **beep = bit 0 of `0x0E09`** (matches CHIRP) |
| Scan rev CO → SE | `0x0E11` | `0x01` → `0x02` | **scan_rev at `0x0E11`**, not CHIRP's `0x0E12` |
| Scan rev CO → TO | `0x0E11` | `0x01` → `0x00` | values `0`=TO, `1`=CO, `2`=SE |
| Remember power (menu) | `0x0E35` | `0x00` ↔ `0x01` | **remember_power at `0x0E35`**, not CHIRP's `reps`. API/UI is inverted vs the stored byte: on ↔ `0x00`, off ↔ `0x01`. |

Pinned by `hardware_cps_diffs_match_the_documented_offsets` in
`core/src/qyt/codeplug/settings.rs`. Captures for the last row live in
`docs/reference/QYT/Configs/power-remember/`.

**`scan_rev` and `remember_power` are the instructive disagreements with CHIRP.**
The WP-9900 struct puts `screv` at `0x0E12` and `reps` (tone-burst frequency) at
`0x0E35`. On the KT-5000 those offsets hold scan resume and remember-power
instead. Shipping CHIRP's labels would write the wrong meaning while the UI
claimed success. This is why settings stay confirmed-only.

`remember_power`: when on, applying supply power restores the radio's last
on/off state; when off, applying power always leaves the radio off. The byte at
`0x0E35` is stored inverted relative to that API value (`true` ↔ `0x00`,
`false` ↔ `0x01`).

Everything from `0x0E36` to `0x0FFF` is **byte-preserved and not modelled**.

This is deliberate. CHIRP sets `has_settings = False` for the KT5000, and its own class
docstring states that the memory map inherited from the WP-9900 is wrong for this
model, giving the example that setting "C display mode" overwrites VOX. Three fields
(`pttid`, `anil`, `tmrtx`) hold values in our capture that are out of range for the
lists CHIRP assigns them, which CHIRP silently clamps before display. Shipping those
labels would mean writing a field the user did not intend to change.

Further names still come from the same hardware one-field-diff pass: change one setting
at a time, dump, and diff. `testUnmodelledSettingsBytesSurviveAnEdit` guarantees the
unmodelled bytes survive an edit untouched.

---

## Hardware verification, 2026-08-06

Run against a real KT-5000 over `/dev/cu.usbserial-10`.

| Check | Result |
|---|---|
| Identify | `KT-500 WP9114`, matching the captures |
| Full read | 12800 bytes, 201 blocks, parses to 159 channels (max index 165) |
| Read vs vendor capture | identical except 2 settings bytes the owner had changed, plus `0x2000`-`0x23EF` (see below) |
| Full write | 784 blocks accepted |
| Read-back vs pre-write backup | **exactly 6 bytes differ**, the intended channel rename at `0x1A50` |
| Read-back vs transmitted image | **0 bytes differ** below the write limit |
| `0x3100`-`0x31FF` | untouched, as intended |
| `0x2000`-`0x23EF` | preserved exactly |

Three bugs only hardware could find, all fixed:

1. **The handshake was over-strict.** The `A1 20 …` prefix is a port-state frame, not
   part of the identity reply, and this radio did not send one. Now optional. See the
   note on [`STATUS_LEN`].
2. **The write path was broken.** `write_verified_image` called `identify()` and then
   `write_image()`, which is two sessions and therefore two handshakes. The KT-5000
   accepts exactly one per open link, so every write would have failed. The identity
   check now happens inside the write session, and the mock enforces one handshake per
   link so this cannot regress silently.
3. **The `0x2000`-`0x23EF` assumption was wrong**, as described in the write policy
   section above.

A second pass the same day confirmed the channel flag bits by changing power, scan-skip,
and BCL one field at a time and diffing. See "Flag bits confirmed on hardware" above.
Bandwidth remains the one unconfirmed flag.

A third pass confirmed three settings bytes via config-only writes (beep, scan_rev,
squelch). See "Settings confirmed on hardware" above. `scan_rev` sits at `0x0E11`, one
byte earlier than CHIRP's WP-9900 map.

A fourth pass on 2026-08-14 confirmed `remember_power` at `0x0E35` (CHIRP's `reps`
slot) by flipping the menu setting on the radio and dumping before/after.

## Known unknowns

| Item | Status |
|---|---|
| USB VID/PID | **There is none to find.** Confirmed 2026-08-06: the cable enumerates as `067b:2303`, a generic Prolific PL2303 shared with thousands of unrelated devices. Attributing that to the KT-5000 would claim every PL2303 adapter is this radio, so the explicit model picker is the correct permanent design, not a stopgap. `KNOWN_USB_IDS` deliberately has no QYT entry. |
| Byte 15 bits 5-4 (`unknown6`) | Unnamed in CHIRP too, and cleared by the vendor software when a channel is edited, so probably vestigial. Round-tripped opaquely regardless. |
| Settings `0x0E36`-`0x0FFF` | Partially modelled through `0x0E35`; the rest awaits further one-field diffs. |
| DCS inverted branch | CHIRP only, no capture example. |
| `0x3000`-`0x30FF` | Believed DTMF/autodial memories. Not modelled. |
| `0x3DF0` prologue purpose | Unknown. Reproduced because it is observed. |
| Channel flag bits: power, scan-add, BCL | **Confirmed on hardware 2026-08-06** by a one-field-at-a-time CPS diff. `scan-add` is writable; the other two are read-only. |
| Channel flag bit: bandwidth | Still CHIRP-derived. No channel had its bandwidth changed during the hardware pass. |
| Settings: beep, scan_rev, squelch, remember_power | **Confirmed on hardware** by one-field diffs. `scan_rev` and `remember_power` offsets disagree with CHIRP. |
