# AT-D878UV(II) Codeplug Memory Map (Phase 2 reference)

Collected from reald/anytone-flash-tools and to be cross-checked against qdmr's
`D878UVCodeplug` class during Phase 2 implementation. **These are D878UV offsets;
the D878UVII may differ in a few regions — verify each against a real backup read
from the target radio before trusting it for writes.** All addresses are the radio
memory addresses used with the `R`/`W` block protocol (see PLAN.md §2).

## Regions

| Region | Base address | Layout |
|--------|--------------|--------|
| Channels | 0x00800000 – 0x00FC0000 | 32 banks × 8192 B (last 2176 B); **64 B per channel** |
| Zones | 0x01000000 | 250 zones × 512 B; channel IDs are 2-byte entries |
| Roaming channels | 0x01040000 | 32 B each, max 250; used-flags bitfield @ 0x01042000 |
| Roaming zones | 0x01043000 | 128 B each, max 64; used-flags @ 0x01042080 |
| Scan lists | 0x01080000 – 0x01441200 | 250 × 144 B at irregular addrs (formula below) |
| SMS management | 0x01640000 | 100 × 16 B; used-flags @ 0x01640800 |
| SMS text | 0x02140000 – 0x02440300 | 100 × 208 B |
| FM (broadcast) channels | 0x02480000 | 100 × 4 B; VFO @ 0x02480200; flags @ 0x02480210 |
| 5-Tone | 0x024C0000 | 100 × 32 B; used-flags @ 0x024C0C80 |
| 2-Tone encode | 0x024C1100 | 24 × 16 B |
| 2-Tone decode | 0x024C2400 | 24 × 32 B |
| Radio settings | 0x02500000 – 0x025014FF | power-on/display/audio/key bindings/etc. |
| APRS config | 0x02501000 – 0x02501490 | general, TX text, GPS templates, freqs |

## DMR regions (implemented; offsets from qdmr `d868uv_codeplug.hh` / `anytone_codeplug.hh`)

These are the offsets the model in `core/src/codeplug/` actually uses. Element
field layouts are in each module's header doc comment.

| Region | Base / bitmap | Layout |
|--------|---------------|--------|
| Digital contacts | banks 0x02680000 (10 × 0x40000, 1000 × 0x64 each) | `ContactElement` 0x64: type@0x00, name@0x01 (16), number@0x23 (BCD8-be), alert@0x27 |
| Contact bitmap | 0x02640000, 0x500 | **inverted** (`InvertedBitmapElement`): active = bit *clear* |
| Contact index | 0x02600000, 10000 × u32-le | dense list of active contact indices |
| Contact ID table | 0x04340000, 10000 × 8 | `ContactMapElement`: id = (bcd_le(number)<<1)\|group_flag @0x00, index @0x04; sorted ascending by number |
| RX group lists | 0x02980000, 250 × 0x200 (0x120 element) | members@0x00 = 64 × u32-le contact indices (0xffffffff empty), name@0x100 (16) |
| Group-list bitmap | 0x025C0B10, 0x20 | normal |
| Radio IDs | 0x02580000, 250 × 0x20 | number@0x00 (BCD8-be), name@0x05 (16) |
| Radio-ID bitmap | 0x024C1320, 0x20 | normal |
| Zone channel list (**read-only**) | 0x02500100, 0x400 | `ZoneChannelListElement`: VFO A@0x000, VFO B@0x200, each 250 × u16-le channel index; 0xffff = unset |

## Never write the radio-settings block (0x02500000 – 0x025014FF)

The zone channel list address above is a **D878UV** offset that lands inside the
radio-settings block, which also holds the power-on password flag and the menu
language. It was briefly added to the write path so zone up/down would follow
zone membership. On a real D878UVII that write locked the radio: it came up
demanding a power-on password nobody had set, with its menus switched to
Chinese. So either the offset is wrong for this model, the element is a
different size here, or neighbouring bytes carry meaning the model does not
capture.

The region stays in `REGIONS` so backups capture the bytes and existing `.bin`
files still parse, but `Radio::write_codeplug` skips it, `serialize` passes it
through untouched, and two tests pin that down
(`serialize_never_touches_the_radio_settings_block`,
`write_codeplug_never_writes_the_radio_settings_block`).

Nothing in this range may be added to the write path until the layout is
confirmed against a real D878UVII backup — a wrong byte here locks the operator
out of the radio, and the tool cannot undo it because it never captured the
original settings.

Channel record DMR fields (64-byte `ChannelElement`): contactIndex@0x14 (u32-le),
radioIdIndex@0x18 (u8), scanListIndex@0x1b (u8), groupListIndex@0x1c (u8),
colorCode@0x20 (u8), timeSlot@{0x21,0}.

The contact index + ID table are derived reverse-lookup structures the radio uses
to show a caller's name by DMR number; the model rebuilds them from the active
contacts (matching qdmr's `encodeContacts`) only when a contact is added, removed,
or renumbered — so unrelated edits never disturb those regions.

## Scan list address formula
```
addr(pos) = 0x01080000 + floor((pos-1)/16) * 0x40000 + ((pos-1) mod 16) * 0x200
```
(pos is 1-based, 1..=250, each record 144 B.)

## Notes for the parser
- Read is done in 16-byte blocks; the codeplug model should operate on assembled
  region byte-slices, not raw blocks.
- Many regions have parallel "used-flags" bitfields — the model must honor these
  when deciding which records are active, and keep them consistent on serialize.
- Round-trip fidelity is the acceptance test: parse → serialize must be byte-
  identical to a codeplug read from the radio (only intentionally edited bytes change).

## Sources
- https://github.com/reald/anytone-flash-tools/blob/master/at-d878uv_memory.md
- https://static.dm3mat.de/qdmr/libdmrconf/classD878UVCodeplug.html
