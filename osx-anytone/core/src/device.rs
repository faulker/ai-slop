//! High-level radio operations built on the [`protocol`] primitives: program
//! mode entry/exit, identify, region reads/writes, and whole-codeplug backup
//! and restore. Transfers are sparse and bitmap-driven, matching the vendor CPS:
//! only active records move, written in address order. The radio commits the
//! codeplug on program-mode exit, so `write_codeplug` relies on each block's
//! write ACK rather than an immediate read-back (which would return pre-commit
//! data); [`Radio::write_region`] still offers verified writes for callers that
//! want them.

use std::collections::BTreeSet;

use crate::codeplug::{
    channel_addr, contact_block_addr, global_offset, group_list_addr, radio_id_addr,
    zone_channels_addr, zone_name_addr, Codeplug, BETWEEN_CONTACT_BANKS, CHANNEL_BITMAP,
    CHANNEL_SIZE, CONTACT_BANKS, CONTACT_BITMAP, CONTACT_BLOCK, CONTACT_ID_TABLE, CONTACT_INDEX,
    GROUP_LISTS, GROUP_LIST_BITMAP, GROUP_LIST_ELEMENT, NUM_CHANNELS, NUM_CONTACTS,
    NUM_GROUP_LISTS, NUM_RADIO_IDS, NUM_ZONES, RADIO_IDS, RADIO_ID_BITMAP, RADIO_ID_SIZE,
    ZONE_BITMAP, ZONE_CHANNELS_SLOT, ZONE_NAME_SLOT,
};
use crate::error::{Error, Result};
use crate::protocol::{
    enter_program_mode, exit_program_mode, identify, read_block, write_block, BLOCK_SIZE,
};
use crate::transport::Transport;

/// A contiguous span of radio memory. The codeplug is the ordered concatenation
/// of all regions' contents.
#[derive(Debug, Clone, Copy)]
pub struct Region {
    /// Big-endian start address in radio memory.
    pub address: u32,
    /// Byte length; must be a multiple of [`BLOCK_SIZE`].
    pub length: usize,
}

/// Real D878UVII codeplug region map.
///
/// Addresses/sizes are ported from qdmr's `D868UVCodeplug::Offset` /
/// `::Limit` and the `AnytoneCodeplug::*Element::size()` constants (the D878UV
/// channel element extends the D868UV one and keeps this layout). Cross-checked
/// against `docs/codeplug-memory-map.md`.
///
/// Covered regions (radio address · length · qdmr source):
/// - Channel banks: 0x00800000, 32 banks spaced 0x00040000, 0x2000 (128 ch ×
///   64 B) each — `Offset::channelBanks/betweenChannelBanks`,
///   `Limit::channelsPerBank/numChannels`, `ChannelElement::size` = 0x40.
/// - Zone channel lists: 0x01000000, 250 × 0x200 — `Offset::zoneChannels`,
///   `Size::zoneChannels`, `Limit::numZones`.
/// - Zone valid-bitmap: 0x024c1300, 0x20 — `Offset::zoneBitmap`,
///   `ZoneBitmapElement::size`.
/// - Channel valid-bitmap: 0x024c1500, 0x200 — `Offset::channelBitmap`,
///   `ChannelBitmapElement::size`.
/// - Zone names: 0x02540000, 250 × 0x20 — `Offset::zoneNames`, `Size::zoneName`.
/// - Contact banks: 0x02680000, 10 banks spaced 0x00040000, 0x186a0 (1000
///   contacts × 100 B) each — `Offset::contactBanks/betweenContactBanks`,
///   `Limit::contactsPerBank/numContacts`, `ContactElement::size` = 0x64.
/// - Contact valid-bitmap (inverted): 0x02640000, 0x500 — `Offset::contactBitmap`,
///   `ContactBitmapElement::size`.
/// - Contact index (u32-le list): 0x02600000, 10000 × 4 — `Offset::contactIndex`.
/// - Contact ID table (id→index map): 0x04340000, 10000 × 8 —
///   `Offset::contactIdTable`, `ContactMapElement::size` = 0x08.
/// - RX group lists: 0x02980000, 250 × 0x200 (0x120 element in a 0x200 slot) —
///   `Offset::groupLists/betweenGroupLists`, `GroupListElement::size`.
/// - Group-list valid-bitmap: 0x025c0b10, 0x20 — `Offset::groupListBitmap`,
///   `GroupListBitmapElement::size`.
/// - Radio IDs: 0x02580000, 250 × 0x20 — `Offset::radioIDs`,
///   `RadioIDElement::size`.
/// - Radio-ID valid-bitmap: 0x024c1320, 0x20 — `Offset::radioIDBitmap`,
///   `RadioIDBitmapElement::size`.
///
/// Every length is a multiple of the 16-byte block size (asserted in a test).
///
/// Note: growing this map grows [`codeplug_size`], so codeplug `.bin` files
/// captured before a region was added will no longer parse.
pub const REGIONS: &[Region] = &REGION_TABLE;

/// Number of channel banks (32 banks × 128 channels ≥ 4000 channels).
const CHANNEL_BANKS: usize = 32;
/// Number of contact banks (10 banks × 1000 contacts = 10000 contacts).
const CONTACT_BANK_COUNT: usize = 10;
/// Fixed regions after the channel banks (zones + bitmaps + DMR entities).
const FIXED_REGIONS: usize = 11;

/// Backing storage for [`REGIONS`], built at compile time so the channel and
/// contact banks don't have to be written out by hand.
const REGION_TABLE: [Region; CHANNEL_BANKS + CONTACT_BANK_COUNT + FIXED_REGIONS] = build_regions();

/// Assemble the region table: the channel banks, then zones and their bitmaps,
/// then the DMR contact banks + reverse-lookup tables, group lists, and radio
/// IDs with their bitmaps.
const fn build_regions() -> [Region; CHANNEL_BANKS + CONTACT_BANK_COUNT + FIXED_REGIONS] {
    let mut arr = [Region {
        address: 0,
        length: 0,
    }; CHANNEL_BANKS + CONTACT_BANK_COUNT + FIXED_REGIONS];
    let mut i = 0;
    while i < CHANNEL_BANKS {
        arr[i] = Region {
            address: 0x0080_0000 + (i as u32) * 0x0004_0000,
            length: 0x2000,
        };
        i += 1;
    }
    // Zone channel lists: 250 zones × 0x200 bytes.
    arr[CHANNEL_BANKS] = Region {
        address: 0x0100_0000,
        length: 250 * 0x200,
    };
    // Zone valid-bitmap.
    arr[CHANNEL_BANKS + 1] = Region {
        address: 0x024c_1300,
        length: 0x20,
    };
    // Channel valid-bitmap.
    arr[CHANNEL_BANKS + 2] = Region {
        address: 0x024c_1500,
        length: 0x200,
    };
    // Zone names: 250 zones × 0x20 bytes.
    arr[CHANNEL_BANKS + 3] = Region {
        address: 0x0254_0000,
        length: 250 * 0x20,
    };
    // Contact valid-bitmap (inverted: active = bit clear).
    arr[CHANNEL_BANKS + 4] = Region {
        address: 0x0264_0000,
        length: 0x500,
    };
    // Contact index: 10000 × u32-le.
    arr[CHANNEL_BANKS + 5] = Region {
        address: 0x0260_0000,
        length: 10000 * 4,
    };
    // Contact ID table: 10000 × 8-byte id→index map entries.
    arr[CHANNEL_BANKS + 6] = Region {
        address: 0x0434_0000,
        length: 10000 * 8,
    };
    // RX group lists: 250 × 0x200 slots (0x120 element each).
    arr[CHANNEL_BANKS + 7] = Region {
        address: 0x0298_0000,
        length: 250 * 0x200,
    };
    // Group-list valid-bitmap.
    arr[CHANNEL_BANKS + 8] = Region {
        address: 0x025c_0b10,
        length: 0x20,
    };
    // Radio IDs: 250 × 0x20 bytes.
    arr[CHANNEL_BANKS + 9] = Region {
        address: 0x0258_0000,
        length: 250 * 0x20,
    };
    // Radio-ID valid-bitmap.
    arr[CHANNEL_BANKS + 10] = Region {
        address: 0x024c_1320,
        length: 0x20,
    };
    // Contact banks: 10 banks × 1000 contacts × 0x64.
    let mut b = 0;
    while b < CONTACT_BANK_COUNT {
        arr[CHANNEL_BANKS + FIXED_REGIONS + b] = Region {
            address: 0x0268_0000 + (b as u32) * 0x0004_0000,
            length: 1000 * 0x64,
        };
        b += 1;
    }
    arr
}

/// Total number of bytes a full codeplug backup/restore moves: the sum of all
/// region lengths.
pub fn codeplug_size() -> usize {
    REGIONS.iter().map(|r| r.length).sum()
}

/// Owns a [`Transport`] and exposes the high-level programming operations.
pub struct Radio<T: Transport> {
    transport: T,
}

impl<T: Transport> Radio<T> {
    /// Wrap a transport. The caller is responsible for opening the serial port.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Consume the radio and return the underlying transport (useful in tests).
    pub fn into_transport(self) -> T {
        self.transport
    }

    /// Enter program mode (`"PROGRAM"` handshake). Must precede identify/reads.
    pub fn enter(&mut self) -> Result<()> {
        enter_program_mode(&mut self.transport)
    }

    /// Exit program mode (`"END"` handshake). Call once when finished.
    pub fn exit(&mut self) -> Result<()> {
        exit_program_mode(&mut self.transport)
    }

    /// Send the identify command and return the model/version string. Must be
    /// called while in program mode.
    pub fn identify(&mut self) -> Result<String> {
        identify(&mut self.transport)
    }

    /// Read `length` bytes starting at `addr` as a sequence of 16-byte blocks.
    /// `length` must be a multiple of [`BLOCK_SIZE`].
    pub fn read_region(&mut self, addr: u32, length: usize) -> Result<Vec<u8>> {
        if !length.is_multiple_of(BLOCK_SIZE) {
            return Err(Error::InvalidArgument(format!(
                "region length {length} is not a multiple of {BLOCK_SIZE}"
            )));
        }
        let mut out = Vec::with_capacity(length);
        let mut offset = 0usize;
        while offset < length {
            let block = read_block(&mut self.transport, addr + offset as u32)?;
            out.extend_from_slice(&block);
            offset += BLOCK_SIZE;
        }
        Ok(out)
    }

    /// Write `data` starting at `addr` as 16-byte blocks, reading each block
    /// back immediately and erroring on any mismatch. `data.len()` must be a
    /// multiple of [`BLOCK_SIZE`].
    pub fn write_region(&mut self, addr: u32, data: &[u8]) -> Result<()> {
        if !data.len().is_multiple_of(BLOCK_SIZE) {
            return Err(Error::InvalidArgument(format!(
                "region length {} is not a multiple of {BLOCK_SIZE}",
                data.len()
            )));
        }
        let mut offset = 0usize;
        while offset < data.len() {
            let block_addr = addr + offset as u32;
            let mut block = [0u8; BLOCK_SIZE];
            block.copy_from_slice(&data[offset..offset + BLOCK_SIZE]);
            write_block(&mut self.transport, block_addr, &block)?;
            // Mandatory read-back verification: never trust a write blindly.
            let readback = read_block(&mut self.transport, block_addr)?;
            if readback != block {
                return Err(Error::Verify { addr: block_addr });
            }
            offset += BLOCK_SIZE;
        }
        Ok(())
    }

    /// Read the codeplug into a full-size buffer, but transfer it the way the
    /// radio actually maps its memory: the channels, zones, and every valid
    /// bitmap are read in full (contiguous config RAM), while the DMR data (the
    /// contact banks, RX group lists, and radio IDs) is read **sparsely** —
    /// only the records the bitmaps mark active. This avoids sending block
    /// commands for memory the radio does not map, which otherwise times out
    /// (see [`Error::Transfer`]). The derived contact index / ID table are not
    /// read back (they are rebuilt on write). `progress` is called after each
    /// block. Assumes program mode has already been entered.
    pub fn read_codeplug(&mut self, progress: &mut dyn FnMut(usize, usize)) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; codeplug_size()];

        // Phase 1: read the DMR bitmaps first so the active records are known.
        // They are re-read in the full-region pass below; the few blocks cost
        // nothing and this keeps the reported progress total exact.
        let mut warmup = |_: usize, _: usize| {};
        for bm in [CONTACT_BITMAP, GROUP_LIST_BITMAP, RADIO_ID_BITMAP] {
            let len = region_length(bm).expect("bitmap region is in REGIONS");
            let mut dummy = 0usize;
            self.read_segment(&mut buf, bm, len, &mut dummy, 0, &mut warmup)?;
        }

        // Build the ordered transfer list: full regions, then active DMR records.
        let mut segments: Vec<(u32, usize)> = REGIONS
            .iter()
            .filter(|r| is_full_transfer_region(r.address))
            .map(|r| (r.address, r.length))
            .collect();
        segments.extend(active_transfer_segments(&buf));

        let total: usize = segments.iter().map(|(_, len)| len / BLOCK_SIZE).sum();
        let mut done = 0usize;
        for (addr, len) in segments {
            self.read_segment(&mut buf, addr, len, &mut done, total, progress)?;
        }
        Ok(buf)
    }

    /// Write a full-size codeplug buffer back to the radio **sparsely**, exactly
    /// as the vendor CPS / qdmr do: the valid bitmaps are written in full (they
    /// are small), and then only the *active* records — channels, zones,
    /// contacts, group lists, radio IDs — are written, followed by the contact
    /// index / ID table sized to the contact count. This is essential: writing
    /// every nominal block (all 32 channel banks, etc.) floods the radio with
    /// tens of thousands of writes to unused slots and resets it mid-transfer.
    ///
    /// Segments are written in **ascending address order** so a record is always
    /// written before the bitmap that references it — the radio commits the
    /// codeplug on program-mode exit, not per block, so it rejects a change that
    /// is internally inconsistent mid-transfer. For the same reason writes are
    /// **not** read-back verified (an immediate read returns the pre-commit
    /// value, so verification would spuriously fail on every genuine edit); each
    /// block's write ACK is the transfer check, exactly as the vendor CPS / qdmr
    /// do. `data.len()` must equal [`codeplug_size`].
    pub fn write_codeplug(
        &mut self,
        data: &[u8],
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<()> {
        let expected = codeplug_size();
        if data.len() != expected {
            return Err(Error::InvalidArgument(format!(
                "codeplug is {} bytes, expected {expected}",
                data.len()
            )));
        }

        // Sized reverse-lookup tables, rebuilt from the active contacts in data.
        let cp = Codeplug::parse(data)?;
        let index_bytes = cp.contact_index_bytes();
        let idtable_bytes = cp.contact_id_table_bytes();

        // Assemble every write segment as (address, bytes): the valid bitmaps,
        // the active records, and the sized contact index / ID table. Sorting by
        // address yields qdmr's write order (records before their bitmaps).
        let mut segments: Vec<(u32, Vec<u8>)> = Vec::new();
        for bm in [
            CHANNEL_BITMAP,
            ZONE_BITMAP,
            CONTACT_BITMAP,
            GROUP_LIST_BITMAP,
            RADIO_ID_BITMAP,
        ] {
            let len = region_length(bm).expect("bitmap region is in REGIONS");
            let off = global_offset(bm).expect("bitmap address is mapped");
            segments.push((bm, data[off..off + len].to_vec()));
        }
        for (addr, len) in active_record_segments(data) {
            let off = global_offset(addr)
                .ok_or_else(|| Error::InvalidArgument(format!("address 0x{addr:08X} unmapped")))?;
            segments.push((addr, data[off..off + len].to_vec()));
        }
        if !index_bytes.is_empty() {
            segments.push((CONTACT_INDEX, index_bytes));
        }
        if !idtable_bytes.is_empty() {
            segments.push((CONTACT_ID_TABLE, idtable_bytes));
        }
        segments.sort_by_key(|(addr, _)| *addr);

        let total: usize = segments.iter().map(|(_, b)| b.len() / BLOCK_SIZE).sum();
        let mut done = 0usize;
        for (addr, bytes) in segments {
            self.write_segment(addr, &bytes, &mut done, total, progress)?;
        }
        Ok(())
    }

    /// Read `len` bytes at `addr` block-by-block into `buf` at the address's
    /// global offset, wrapping any IO failure with the failing block address.
    fn read_segment(
        &mut self,
        buf: &mut [u8],
        addr: u32,
        len: usize,
        done: &mut usize,
        total: usize,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<()> {
        let off = global_offset(addr)
            .ok_or_else(|| Error::InvalidArgument(format!("address 0x{addr:08X} unmapped")))?;
        let mut o = 0usize;
        while o < len {
            let a = addr + o as u32;
            let block = read_block(&mut self.transport, a).map_err(|e| at(a, e))?;
            buf[off + o..off + o + BLOCK_SIZE].copy_from_slice(&block);
            o += BLOCK_SIZE;
            *done += 1;
            progress(*done, total);
        }
        Ok(())
    }

    /// Write `bytes` at `addr` block-by-block, wrapping any IO failure with the
    /// failing block address. When `verify` is true each block is read back and
    /// compared (the safety gate for real codeplug data); pass false for derived
    /// regions the radio does not store verbatim. `bytes.len()` must be a
    /// multiple of [`BLOCK_SIZE`]. Each block's write ACK (checked in
    /// [`write_block`]) is the transfer check; the radio commits on program-mode
    /// exit so blocks are not read back here.
    fn write_segment(
        &mut self,
        addr: u32,
        bytes: &[u8],
        done: &mut usize,
        total: usize,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<()> {
        let mut o = 0usize;
        while o < bytes.len() {
            let a = addr + o as u32;
            let mut block = [0u8; BLOCK_SIZE];
            block.copy_from_slice(&bytes[o..o + BLOCK_SIZE]);
            write_block(&mut self.transport, a, &block).map_err(|e| at(a, e))?;
            o += BLOCK_SIZE;
            *done += 1;
            progress(*done, total);
        }
        Ok(())
    }
}

/// True for regions transferred in full: channels, zones, and every valid
/// bitmap (contiguous config RAM the radio always services). The DMR *data*
/// regions are not here — they are transferred sparsely.
fn is_full_transfer_region(addr: u32) -> bool {
    !is_sparse_data_region(addr)
}

/// True for the DMR data regions that must not be sent as fixed full blocks: the
/// contact banks, contact index, contact ID table, RX group lists, and radio
/// IDs. Their bitmaps are deliberately excluded (bitmaps transfer in full).
fn is_sparse_data_region(addr: u32) -> bool {
    if addr == CONTACT_INDEX || addr == CONTACT_ID_TABLE || addr == GROUP_LISTS || addr == RADIO_IDS
    {
        return true;
    }
    (0..10u32).any(|b| addr == CONTACT_BANKS + b * BETWEEN_CONTACT_BANKS)
}

/// The active *record* write segments for `data` (no bitmaps): only the
/// channels, zones, and DMR families the bitmaps mark active. Mirrors qdmr's
/// `allocateForEncoding` so the radio is never flooded with writes to unused
/// slots. Bitmaps and the sized contact index / ID table are added by the
/// caller, which then writes everything in address order.
fn active_record_segments(data: &[u8]) -> Vec<(u32, usize)> {
    let mut segments = Vec::new();

    // Active channels (one 0x40 element each).
    for i in 0..NUM_CHANNELS {
        if bitmap_active(data, CHANNEL_BITMAP, i, false) {
            segments.push((channel_addr(i), CHANNEL_SIZE));
        }
    }
    // Active zones (name slot + channel-list slot each).
    for i in 0..NUM_ZONES {
        if bitmap_active(data, ZONE_BITMAP, i, false) {
            segments.push((zone_name_addr(i), ZONE_NAME_SLOT));
            segments.push((zone_channels_addr(i), ZONE_CHANNELS_SLOT));
        }
    }

    // Active DMR records (contacts / group lists / radio IDs).
    segments.extend(active_transfer_segments(data));
    segments
}

/// The sparse DMR transfer segments implied by the bitmaps in `buf`: one
/// [`CONTACT_BLOCK`] per distinct block holding an active contact, one element
/// per active group list, and one record per active radio ID.
fn active_transfer_segments(buf: &[u8]) -> Vec<(u32, usize)> {
    let mut segments = Vec::new();

    // Contacts share 4-per-block storage; transfer each distinct block once.
    let mut blocks = BTreeSet::new();
    for i in 0..NUM_CONTACTS {
        if bitmap_active(buf, CONTACT_BITMAP, i, true) && blocks.insert(contact_block_addr(i)) {
            segments.push((contact_block_addr(i), CONTACT_BLOCK));
        }
    }
    for i in 0..NUM_GROUP_LISTS {
        if bitmap_active(buf, GROUP_LIST_BITMAP, i, false) {
            segments.push((group_list_addr(i), GROUP_LIST_ELEMENT));
        }
    }
    for i in 0..NUM_RADIO_IDS {
        if bitmap_active(buf, RADIO_ID_BITMAP, i, false) {
            segments.push((radio_id_addr(i), RADIO_ID_SIZE));
        }
    }
    segments
}

/// Test bit `i` of the bitmap at `bitmap_addr` within `buf`. For an `inverted`
/// bitmap (the contact bitmap) "active" means the bit is clear.
fn bitmap_active(buf: &[u8], bitmap_addr: u32, i: usize, inverted: bool) -> bool {
    let Some(base) = global_offset(bitmap_addr) else {
        return false;
    };
    let set = (buf[base + i / 8] >> (i % 8)) & 1 == 1;
    set != inverted
}

/// Length of the region at radio address `addr`, or `None` if not in [`REGIONS`].
fn region_length(addr: u32) -> Option<usize> {
    REGIONS.iter().find(|r| r.address == addr).map(|r| r.length)
}

/// Annotate a block-transfer error with the failing radio address. Errors that
/// already carry an address (verify/checksum/transfer) pass through unchanged;
/// anything else (notably an IO timeout when the radio does not map the address)
/// is wrapped in [`Error::Transfer`] so the failing block is visible.
fn at(addr: u32, e: Error) -> Error {
    match e {
        Error::Verify { .. } | Error::Checksum { .. } | Error::Transfer { .. } => e,
        other => Error::Transfer {
            addr,
            message: other.to_string(),
        },
    }
}

/// Best-effort check that an identify string looks like a D878UV-family radio.
/// Used as a guard before writing so a codeplug is not pushed to a mismatched
/// model.
pub fn is_supported_model(identify_string: &str) -> bool {
    identify_string.contains("878UV")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    /// Build a mock whose flat memory image is large enough to cover the
    /// highest-addressed region in [`REGIONS`].
    fn mock() -> MockTransport {
        let end = REGIONS
            .iter()
            .map(|r| r.address as usize + r.length)
            .max()
            .unwrap();
        MockTransport::new(end, "ID878UV V100")
    }

    #[test]
    fn every_region_length_is_block_aligned() {
        for r in REGIONS {
            assert_eq!(
                r.length % BLOCK_SIZE,
                0,
                "region at 0x{:08x} length {} is not a multiple of {BLOCK_SIZE}",
                r.address,
                r.length
            );
        }
    }

    #[test]
    fn regions_do_not_overlap() {
        // Sort a copy by address and check each region ends before the next starts.
        let mut spans: Vec<(u32, u32)> = REGIONS
            .iter()
            .map(|r| (r.address, r.address + r.length as u32))
            .collect();
        spans.sort_by_key(|s| s.0);
        for pair in spans.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "region ending at 0x{:08x} overlaps region starting at 0x{:08x}",
                pair[0].1,
                pair[1].0
            );
        }
    }

    #[test]
    fn identify_guard_accepts_known_and_rejects_unknown() {
        assert!(is_supported_model("ID878UV V100"));
        assert!(!is_supported_model("ID578UV"));
    }

    #[test]
    fn read_region_rejects_unaligned_length() {
        let mut r = Radio::new(mock());
        let err = r.read_region(0, 17).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)));
    }

    /// An empty codeplug image: all zero, but with the inverted contact bitmap
    /// set so no contacts read as active.
    fn empty_image() -> Vec<u8> {
        let mut raw = vec![0u8; codeplug_size()];
        let off = global_offset(CONTACT_BITMAP).unwrap();
        for b in raw[off..off + 0x500].iter_mut() {
            *b = 0xff;
        }
        raw
    }

    #[test]
    fn sparse_codeplug_roundtrips_all_entities() {
        // Build a realistic codeplug (a channel, a zone, a contact, a group
        // list, a radio ID) via the model, write it sparsely, read it back, and
        // confirm every entity survives the round trip.
        let mut cp = Codeplug::parse(&empty_image()).unwrap();
        let ci = cp.add_channel().unwrap();
        cp.channel_mut(ci).unwrap().set_name("REPEATER");
        cp.channel_mut(ci).unwrap().set_rx_frequency(146_940_000);
        let ct = cp.add_contact().unwrap();
        cp.set_contact_name(ct, "TAC 310");
        cp.set_contact_number(ct, 310);
        let gl = cp.add_group_list().unwrap();
        cp.group_list_mut(gl).unwrap().set_name("STATE");
        cp.group_list_mut(gl).unwrap().set_members(&[ct as u32]);
        let rid = cp.add_radio_id().unwrap();
        cp.radio_id_mut(rid).unwrap().set_number(3_141_592);
        let plug = cp.serialize();

        let mut radio = Radio::new(mock());
        radio.enter().unwrap();
        let mut noop = |_: usize, _: usize| {};
        radio.write_codeplug(&plug, &mut noop).unwrap();
        let back = radio.read_codeplug(&mut noop).unwrap();
        radio.exit().unwrap();

        let got = Codeplug::parse(&back).unwrap();
        assert_eq!(got.channels().next().unwrap().name, "REPEATER");
        assert_eq!(got.channels().next().unwrap().rx_frequency_hz, 146_940_000);
        let c = got.contacts().next().unwrap();
        assert_eq!((c.name.as_str(), c.number), ("TAC 310", 310));
        assert_eq!(got.group_lists().next().unwrap().members, vec![ct as u32]);
        assert_eq!(got.radio_ids().next().unwrap().number, 3_141_592);
    }

    #[test]
    fn write_codeplug_rejects_wrong_size() {
        let mut radio = Radio::new(mock());
        radio.enter().unwrap();
        let mut noop = |_: usize, _: usize| {};
        let err = radio.write_codeplug(&[0u8; 3], &mut noop).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)));
    }

    #[test]
    fn empty_codeplug_never_touches_the_contact_id_table() {
        // With no contacts, the far contact ID table (0x04340000) must not be
        // written at all — that region is the one the radio does not map, and
        // writing it is what caused the IO timeout.
        let mut radio = Radio::new(mock());
        radio.enter().unwrap();
        let mut noop = |_: usize, _: usize| {};
        radio.write_codeplug(&empty_image(), &mut noop).unwrap();
        // The mock starts zeroed; a bare read of the id table stays zero, i.e.
        // nothing was written there.
        let block = radio.read_region(CONTACT_ID_TABLE, BLOCK_SIZE).unwrap();
        assert_eq!(block, vec![0u8; BLOCK_SIZE]);
        radio.exit().unwrap();
    }

    #[test]
    fn progress_reports_every_transferred_block() {
        let mut radio = Radio::new(mock());
        radio.enter().unwrap();
        let mut count = 0usize;
        let mut last_total = 0usize;
        {
            let mut cb = |done: usize, total: usize| {
                count = done;
                last_total = total;
            };
            radio.read_codeplug(&mut cb).unwrap();
        }
        // The last callback reports the final block, matching the total.
        assert!(last_total > 0);
        assert_eq!(count, last_total);
    }
}
