//! Codeplug binary model: parse the in-memory codeplug byte buffer (the ordered
//! concatenation of [`crate::device::REGIONS`], exactly what
//! [`crate::device::Radio::read_codeplug`] returns) into typed structs, and
//! re-serialize losslessly.
//!
//! # Authoritative offsets and their sources
//!
//! All field offsets/sizes below are ported from qdmr (GPLv3; referenced, not
//! copied) and cross-checked against `docs/codeplug-memory-map.md`. The D878UV
//! channel element extends the D868UV one and keeps the base layout, so the
//! channel field offsets come from the shared base class.
//!
//! Region addresses — `lib/d868uv_codeplug.hh`, `D868UVCodeplug::Offset` /
//! `::Limit` / `::Size`:
//! - `channelBanks()`        = 0x00800000
//! - `betweenChannelBanks()` = 0x00040000
//! - `channelsPerBank()`     = 128, `numChannels()` = 4000
//! - `channelBitmap()`       = 0x024c1500  (`ChannelBitmapElement::size` = 0x200)
//! - `zoneChannels()`        = 0x01000000
//! - `betweenZoneChannels()` = 0x00000200  (`Size::zoneChannels` = 0x200)
//! - `zoneBitmap()`          = 0x024c1300  (`ZoneBitmapElement::size` = 0x20)
//! - `zoneNames()`           = 0x02540000
//! - `betweenZoneNames()`    = 0x00000020  (`Size::zoneName` = 0x20)
//! - `numZones()`            = 250, `numChannelsPerZone()` = 250,
//!   `zoneNameLength()`      = 16
//!
//! Channel record — `lib/anytone_codeplug.hh`,
//! `AnytoneCodeplug::ChannelElement::Offset` (`size()` = 0x40 = 64 B):
//! - rxFrequency 0x0000 (BCD8 big-endian, value×10 Hz)
//! - txFrequencyOffset 0x0004 (BCD8 big-endian, value×10 Hz)
//! - channelMode {0x0008,0} · power {0x0008,2} · bandwidth {0x0008,4} ·
//!   repeaterMode {0x0008,6}  (each a 2-bit field via `getUInt2`)
//! - contactIndex 0x0014 (u32 le) · radioIdIndex 0x0018 (u8)
//! - colorCode 0x0020 (u8) · timeSlot {0x0021,0} (1 bit)
//! - name 0x0023, 16 bytes, Latin1, `0x00`-padded/terminated
//!
//! Encodings — `lib/codeplug.cc`:
//! - `getBCD8_be`/`setBCD8_be`: 8 BCD digits, most-significant nibble first.
//! - `getUInt2(byte,bit)` = `(data[byte] >> bit) & 0b11`.
//! - `BitmapElement::isEncoded(i)` = `data[i/8] & (1 << (i%8))`; set bit = valid
//!   for both the channel and zone bitmaps.
//! - `readASCII`/`writeASCII`: fixed-length, padded/terminated with the fill byte.

pub mod channel;
pub mod contact;
pub mod edits;
pub mod group_list;
pub mod radio_id;
pub mod zone;

use crate::device::REGIONS;
use crate::error::{Error, Result};

pub use channel::{Bandwidth, Channel, ChannelMode, Power, RepeaterMode, CHANNEL_SIZE};
pub use edits::apply_edits;
pub use contact::{CallType, Contact, CONTACT_SIZE};
pub use group_list::{GroupList, GROUP_LIST_ELEMENT, GROUP_LIST_SLOT};
pub use radio_id::{RadioId, RADIO_ID_SIZE};
pub use zone::{Zone, ZONE_CHANNELS_SLOT, ZONE_NAME_SLOT};

use serde::Serialize;

/// Radio address of the first channel bank.
pub const CHANNEL_BANKS: u32 = 0x0080_0000;
/// Address spacing between successive channel banks.
pub const BETWEEN_CHANNEL_BANKS: u32 = 0x0004_0000;
/// Channels stored per bank.
pub const CHANNELS_PER_BANK: usize = 128;
/// Maximum number of channels the radio supports.
pub const NUM_CHANNELS: usize = 4000;
/// Radio address of the channel valid-bitmap (one bit per channel).
pub const CHANNEL_BITMAP: u32 = 0x024c_1500;

/// Radio address of the first zone's channel list.
pub const ZONE_CHANNELS: u32 = 0x0100_0000;
/// Address spacing between successive zone channel lists.
pub const BETWEEN_ZONE_CHANNELS: u32 = 0x0000_0200;
/// Radio address of the first zone's name slot.
pub const ZONE_NAMES: u32 = 0x0254_0000;
/// Address spacing between successive zone name slots.
pub const BETWEEN_ZONE_NAMES: u32 = 0x0000_0020;
/// Radio address of the zone valid-bitmap (one bit per zone).
pub const ZONE_BITMAP: u32 = 0x024c_1300;
/// Maximum number of zones the radio supports.
pub const NUM_ZONES: usize = 250;
/// Maximum number of channel members per zone.
pub const CHANNELS_PER_ZONE: usize = 250;

/// Radio address of the first contact bank.
pub const CONTACT_BANKS: u32 = 0x0268_0000;
/// Address spacing between successive contact banks.
pub const BETWEEN_CONTACT_BANKS: u32 = 0x0004_0000;
/// Contacts stored per bank.
pub const CONTACTS_PER_BANK: usize = 1000;
/// Maximum number of contacts the radio supports.
pub const NUM_CONTACTS: usize = 10000;
/// Radio address of the contact valid-bitmap (inverted: active = bit clear).
pub const CONTACT_BITMAP: u32 = 0x0264_0000;
/// Radio address of the contact index (u32-le list of active contact indices).
pub const CONTACT_INDEX: u32 = 0x0260_0000;
/// Radio address of the contact ID table (id→index map, 8 bytes per entry).
pub const CONTACT_ID_TABLE: u32 = 0x0434_0000;
/// Byte size of one contact ID-table entry (qdmr `ContactMapElement::size`).
pub const CONTACT_MAP_ENTRY: usize = 8;
/// Byte size of a contact storage block (4 contacts × 0x64, qdmr
/// `Offset::betweenContactBlocks`). Contacts are transferred a block at a time.
pub const CONTACT_BLOCK: usize = 0x190;
/// Contacts stored per block (qdmr `Limit::contactsPerBlock`).
pub const CONTACTS_PER_BLOCK: usize = 4;

/// Radio address of the first RX group list.
pub const GROUP_LISTS: u32 = 0x0298_0000;
/// Address spacing between successive group lists.
pub const BETWEEN_GROUP_LISTS: u32 = 0x0000_0200;
/// Radio address of the group-list valid-bitmap (one bit per group list).
pub const GROUP_LIST_BITMAP: u32 = 0x025c_0b10;
/// Maximum number of RX group lists the radio supports.
pub const NUM_GROUP_LISTS: usize = 250;

/// Radio address of the first radio-ID record.
pub const RADIO_IDS: u32 = 0x0258_0000;
/// Radio address of the radio-ID valid-bitmap (one bit per radio ID).
pub const RADIO_ID_BITMAP: u32 = 0x024c_1320;
/// Maximum number of radio IDs the radio supports.
pub const NUM_RADIO_IDS: usize = 250;

/// The parsed codeplug. Holds the full raw byte image (the source of truth for
/// bitmaps and any bytes this first slice does not model) plus typed views of
/// the active channels and zones. [`serialize`](Codeplug::serialize) writes the
/// typed records back over a clone of the raw image, guaranteeing byte-for-byte
/// fidelity for everything untouched.
#[derive(Debug, Clone)]
pub struct Codeplug {
    /// Full concatenated codeplug image, one entry per byte of [`REGIONS`].
    raw: Vec<u8>,
    /// One slot per channel index (`0..NUM_CHANNELS`); `Some` when the channel
    /// valid-bitmap marks it active.
    channels: Vec<Option<Channel>>,
    /// One slot per zone index (`0..NUM_ZONES`); `Some` when the zone
    /// valid-bitmap marks it active.
    zones: Vec<Option<Zone>>,
    /// One slot per contact index (`0..NUM_CONTACTS`); `Some` when the
    /// (inverted) contact valid-bitmap marks it active.
    contacts: Vec<Option<Contact>>,
    /// One slot per group-list index (`0..NUM_GROUP_LISTS`); `Some` when active.
    group_lists: Vec<Option<GroupList>>,
    /// One slot per radio-ID index (`0..NUM_RADIO_IDS`); `Some` when active.
    radio_ids: Vec<Option<RadioId>>,
    /// Set when a contact was added, removed, or had its number/type changed —
    /// the only mutations that require rebuilding the reverse-lookup tables. Any
    /// other edit leaves the contact index / ID table exactly as parsed, so
    /// unrelated saves never disturb those regions.
    contacts_dirty: bool,
}

/// Serializable projection of a [`Codeplug`] used by the CLI `dump` command.
#[derive(Debug, Serialize)]
pub struct CodeplugJson<'a> {
    /// The active channels, in ascending index order.
    pub channels: Vec<&'a Channel>,
    /// The active zones, in ascending index order.
    pub zones: Vec<&'a Zone>,
    /// The active contacts / talk groups, in ascending index order.
    pub contacts: Vec<&'a Contact>,
    /// The active RX group lists, in ascending index order.
    pub group_lists: Vec<&'a GroupList>,
    /// The active radio IDs, in ascending index order.
    pub radio_ids: Vec<&'a RadioId>,
}

impl Codeplug {
    /// Parse a full codeplug image into the typed model. `raw.len()` must equal
    /// [`crate::device::codeplug_size`]; every modeled address must fall inside
    /// [`REGIONS`] or an [`Error::Parse`] is returned.
    pub fn parse(raw: &[u8]) -> Result<Codeplug> {
        let expected = crate::device::codeplug_size();
        if raw.len() != expected {
            return Err(Error::Parse(format!(
                "codeplug is {} bytes, expected {expected}",
                raw.len()
            )));
        }
        let raw = raw.to_vec();

        // Channels: honor the channel valid-bitmap.
        let channel_bitmap = read_region(&raw, CHANNEL_BITMAP, 0x200)?;
        let mut channels = Vec::with_capacity(NUM_CHANNELS);
        for i in 0..NUM_CHANNELS {
            if bit_set(channel_bitmap, i) {
                let off = global_offset(channel_addr(i))
                    .ok_or_else(|| Error::Parse(format!("channel {i} address unmapped")))?;
                let rec = &raw[off..off + CHANNEL_SIZE];
                channels.push(Some(Channel::parse(i, rec)?));
            } else {
                channels.push(None);
            }
        }

        // Zones: honor the zone valid-bitmap.
        let zone_bitmap = read_region(&raw, ZONE_BITMAP, 0x20)?;
        let mut zones = Vec::with_capacity(NUM_ZONES);
        for i in 0..NUM_ZONES {
            if bit_set(zone_bitmap, i) {
                let name_off = global_offset(zone_name_addr(i))
                    .ok_or_else(|| Error::Parse(format!("zone {i} name address unmapped")))?;
                let chan_off = global_offset(zone_channels_addr(i))
                    .ok_or_else(|| Error::Parse(format!("zone {i} channels address unmapped")))?;
                let name = &raw[name_off..name_off + ZONE_NAME_SLOT];
                let chans = &raw[chan_off..chan_off + ZONE_CHANNELS_SLOT];
                zones.push(Some(Zone::parse(i, name, chans)?));
            } else {
                zones.push(None);
            }
        }

        // Contacts: honor the contact valid-bitmap, which is *inverted*
        // (qdmr `ContactBitmapElement` / `InvertedBitmapElement`: a contact is
        // active when its bit is CLEAR).
        let contact_bitmap = read_region(&raw, CONTACT_BITMAP, 0x500)?;
        let mut contacts = Vec::with_capacity(NUM_CONTACTS);
        for i in 0..NUM_CONTACTS {
            if bit_clear(contact_bitmap, i) {
                let off = global_offset(contact_addr(i))
                    .ok_or_else(|| Error::Parse(format!("contact {i} address unmapped")))?;
                let rec = &raw[off..off + CONTACT_SIZE];
                contacts.push(Some(Contact::parse(i, rec)?));
            } else {
                contacts.push(None);
            }
        }

        // RX group lists: honor the (normal) group-list valid-bitmap.
        let group_bitmap = read_region(&raw, GROUP_LIST_BITMAP, 0x20)?;
        let mut group_lists = Vec::with_capacity(NUM_GROUP_LISTS);
        for i in 0..NUM_GROUP_LISTS {
            if bit_set(group_bitmap, i) {
                let off = global_offset(group_list_addr(i))
                    .ok_or_else(|| Error::Parse(format!("group list {i} address unmapped")))?;
                let rec = &raw[off..off + GROUP_LIST_SLOT];
                group_lists.push(Some(GroupList::parse(i, rec)?));
            } else {
                group_lists.push(None);
            }
        }

        // Radio IDs: honor the (normal) radio-ID valid-bitmap.
        let radio_bitmap = read_region(&raw, RADIO_ID_BITMAP, 0x20)?;
        let mut radio_ids = Vec::with_capacity(NUM_RADIO_IDS);
        for i in 0..NUM_RADIO_IDS {
            if bit_set(radio_bitmap, i) {
                let off = global_offset(radio_id_addr(i))
                    .ok_or_else(|| Error::Parse(format!("radio ID {i} address unmapped")))?;
                let rec = &raw[off..off + RADIO_ID_SIZE];
                radio_ids.push(Some(RadioId::parse(i, rec)?));
            } else {
                radio_ids.push(None);
            }
        }

        Ok(Codeplug {
            raw,
            channels,
            zones,
            contacts,
            group_lists,
            radio_ids,
            contacts_dirty: false,
        })
    }

    /// Re-serialize the model to a full codeplug image. Untouched bytes come
    /// straight from the parsed image; each active channel/zone's record is
    /// written back over its slot. With no edits the result is byte-identical to
    /// the input of [`parse`](Codeplug::parse).
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = self.raw.clone();

        // Channels + their (normal) valid-bitmap.
        for (i, slot) in self.channels.iter().enumerate() {
            if let Some(ch) = slot {
                // `channel_addr(i)` is known-mapped: it was validated at parse.
                if let Some(off) = global_offset(channel_addr(i)) {
                    out[off..off + CHANNEL_SIZE].copy_from_slice(ch.encode());
                }
            }
            write_bit(&mut out, CHANNEL_BITMAP, i, slot.is_some(), false);
        }

        // Zones + their (normal) valid-bitmap.
        for (i, slot) in self.zones.iter().enumerate() {
            if let Some(zone) = slot {
                if let Some(off) = global_offset(zone_name_addr(i)) {
                    out[off..off + ZONE_NAME_SLOT].copy_from_slice(zone.name_bytes());
                }
                if let Some(off) = global_offset(zone_channels_addr(i)) {
                    out[off..off + ZONE_CHANNELS_SLOT].copy_from_slice(zone.channel_bytes());
                }
            }
            write_bit(&mut out, ZONE_BITMAP, i, slot.is_some(), false);
        }

        // Contacts + their INVERTED valid-bitmap (active = bit clear).
        for (i, slot) in self.contacts.iter().enumerate() {
            if let Some(c) = slot {
                if let Some(off) = global_offset(contact_addr(i)) {
                    out[off..off + CONTACT_SIZE].copy_from_slice(c.encode());
                }
            }
            write_bit(&mut out, CONTACT_BITMAP, i, slot.is_some(), true);
        }

        // RX group lists + their (normal) valid-bitmap.
        for (i, slot) in self.group_lists.iter().enumerate() {
            if let Some(gl) = slot {
                if let Some(off) = global_offset(group_list_addr(i)) {
                    out[off..off + GROUP_LIST_SLOT].copy_from_slice(gl.encode());
                }
            }
            write_bit(&mut out, GROUP_LIST_BITMAP, i, slot.is_some(), false);
        }

        // Radio IDs + their (normal) valid-bitmap.
        for (i, slot) in self.radio_ids.iter().enumerate() {
            if let Some(rid) = slot {
                if let Some(off) = global_offset(radio_id_addr(i)) {
                    out[off..off + RADIO_ID_SIZE].copy_from_slice(rid.encode());
                }
            }
            write_bit(&mut out, RADIO_ID_BITMAP, i, slot.is_some(), false);
        }

        // The contact reverse-lookup tables are derived data. Only rebuild them
        // when contacts actually changed, so unrelated saves leave them exactly
        // as read from the radio.
        if self.contacts_dirty {
            self.rebuild_contact_tables(&mut out);
        }

        out
    }

    /// Regenerate the contact index (`CONTACT_INDEX`) and ID table
    /// (`CONTACT_ID_TABLE`) from the active contacts, matching qdmr's
    /// `encodeContacts`: the index is a dense u32-le list of active contact
    /// slot indices; the ID table is `ContactMapElement` entries (id with a
    /// group-call flag in bit 0, plus the slot index) sorted ascending by DMR
    /// number. Unused index entries are zeroed; unused table entries are 0xff.
    fn rebuild_contact_tables(&self, out: &mut [u8]) {
        let active: Vec<&Contact> = self.contacts().collect();

        // Contact index: dense list of active slot indices, rest zeroed.
        if let Some(base) = global_offset(CONTACT_INDEX) {
            for slot in out[base..base + NUM_CONTACTS * 4].iter_mut() {
                *slot = 0;
            }
            for (k, c) in active.iter().enumerate() {
                let off = base + k * 4;
                out[off..off + 4].copy_from_slice(&(c.index as u32).to_le_bytes());
            }
        }

        // Contact ID table: id→index map sorted ascending by number, rest 0xff.
        if let Some(base) = global_offset(CONTACT_ID_TABLE) {
            for slot in out[base..base + NUM_CONTACTS * CONTACT_MAP_ENTRY].iter_mut() {
                *slot = 0xff;
            }
            let mut sorted = active.clone();
            sorted.sort_by_key(|c| c.number);
            for (k, c) in sorted.iter().enumerate() {
                let off = base + k * CONTACT_MAP_ENTRY;
                let is_group = matches!(c.call_type, CallType::Group);
                let id = (encode_dmr_id_bcd_le(c.number) << 1) | (is_group as u32);
                out[off..off + 4].copy_from_slice(&id.to_le_bytes());
                out[off + 4..off + 8].copy_from_slice(&(c.index as u32).to_le_bytes());
            }
        }
    }

    /// The contact index as it is transferred to the radio: a dense u32-le list
    /// of active contact indices, sized to the contact count and padded up to a
    /// 16-byte boundary with `0xff` (qdmr `0xff`-fills the region and overwrites
    /// only the entry bytes, so the tail padding must be `0xff` to match what
    /// the radio stores). Empty when there are no contacts.
    pub fn contact_index_bytes(&self) -> Vec<u8> {
        let active: Vec<&Contact> = self.contacts().collect();
        if active.is_empty() {
            return Vec::new();
        }
        let mut out = vec![0xff; align_up(active.len() * 4, 16)];
        for (k, c) in active.iter().enumerate() {
            out[k * 4..k * 4 + 4].copy_from_slice(&(c.index as u32).to_le_bytes());
        }
        out
    }

    /// The contact ID table as it is transferred to the radio: `ContactMapElement`
    /// entries (id with a group-call flag in bit 0, plus the slot index) sorted
    /// ascending by DMR number, followed by a `0xff` terminator entry, padded to
    /// a 16-byte boundary. Empty when there are no contacts.
    pub fn contact_id_table_bytes(&self) -> Vec<u8> {
        let mut active: Vec<&Contact> = self.contacts().collect();
        if active.is_empty() {
            return Vec::new();
        }
        active.sort_by_key(|c| c.number);
        // count entries + one 0xff terminator entry, padded to 16 bytes.
        let len = align_up((active.len() + 1) * CONTACT_MAP_ENTRY, 16);
        let mut out = vec![0xff; len];
        for (k, c) in active.iter().enumerate() {
            let off = k * CONTACT_MAP_ENTRY;
            let is_group = matches!(c.call_type, CallType::Group);
            let id = (encode_dmr_id_bcd_le(c.number) << 1) | (is_group as u32);
            out[off..off + 4].copy_from_slice(&id.to_le_bytes());
            out[off + 4..off + 8].copy_from_slice(&(c.index as u32).to_le_bytes());
        }
        out
    }

    /// The active contact indices in ascending order (for the sparse transfer
    /// layer, which reads/writes only the blocks that hold active contacts).
    pub fn active_contact_indices(&self) -> Vec<usize> {
        self.contacts().map(|c| c.index).collect()
    }

    /// The active group-list indices in ascending order.
    pub fn active_group_list_indices(&self) -> Vec<usize> {
        self.group_lists().map(|g| g.index).collect()
    }

    /// The active radio-ID indices in ascending order.
    pub fn active_radio_id_indices(&self) -> Vec<usize> {
        self.radio_ids().map(|r| r.index).collect()
    }

    /// Iterate the active channels in ascending index order.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.iter().filter_map(|c| c.as_ref())
    }

    /// Iterate the active zones in ascending index order.
    pub fn zones(&self) -> impl Iterator<Item = &Zone> {
        self.zones.iter().filter_map(|z| z.as_ref())
    }

    /// Mutable access to an active channel by index, or `None` if the index is
    /// out of range or the slot is inactive.
    pub fn channel_mut(&mut self, index: usize) -> Option<&mut Channel> {
        self.channels.get_mut(index).and_then(|c| c.as_mut())
    }

    /// Mutable access to an active zone by index, or `None` if the index is out
    /// of range or the slot is inactive.
    pub fn zone_mut(&mut self, index: usize) -> Option<&mut Zone> {
        self.zones.get_mut(index).and_then(|z| z.as_mut())
    }

    /// Iterate the active contacts in ascending index order.
    pub fn contacts(&self) -> impl Iterator<Item = &Contact> {
        self.contacts.iter().filter_map(|c| c.as_ref())
    }

    /// Iterate the active group lists in ascending index order.
    pub fn group_lists(&self) -> impl Iterator<Item = &GroupList> {
        self.group_lists.iter().filter_map(|g| g.as_ref())
    }

    /// Iterate the active radio IDs in ascending index order.
    pub fn radio_ids(&self) -> impl Iterator<Item = &RadioId> {
        self.radio_ids.iter().filter_map(|r| r.as_ref())
    }

    /// Mutable access to an active group list by index.
    pub fn group_list_mut(&mut self, index: usize) -> Option<&mut GroupList> {
        self.group_lists.get_mut(index).and_then(|g| g.as_mut())
    }

    /// Mutable access to an active radio ID by index.
    pub fn radio_id_mut(&mut self, index: usize) -> Option<&mut RadioId> {
        self.radio_ids.get_mut(index).and_then(|r| r.as_mut())
    }

    /// Read-only access to an active contact by index.
    pub fn contact(&self, index: usize) -> Option<&Contact> {
        self.contacts.get(index).and_then(|c| c.as_ref())
    }

    // --- Contact edits go through the codeplug so the reverse-lookup tables are
    // rebuilt only when a change actually affects them (name edits do not). ---

    /// Rename an active contact. Does not dirty the lookup tables (the name is
    /// not part of them). Returns false if the contact is not active.
    pub fn set_contact_name(&mut self, index: usize, name: &str) -> bool {
        match self.contacts.get_mut(index).and_then(|c| c.as_mut()) {
            Some(c) => {
                c.set_name(name);
                true
            }
            None => false,
        }
    }

    /// Set an active contact's DMR number; dirties the lookup tables.
    pub fn set_contact_number(&mut self, index: usize, number: u32) -> bool {
        match self.contacts.get_mut(index).and_then(|c| c.as_mut()) {
            Some(c) => {
                c.set_number(number);
                self.contacts_dirty = true;
                true
            }
            None => false,
        }
    }

    /// Set an active contact's call type; dirties the lookup tables.
    pub fn set_contact_call_type(&mut self, index: usize, call_type: CallType) -> bool {
        match self.contacts.get_mut(index).and_then(|c| c.as_mut()) {
            Some(c) => {
                c.set_call_type(call_type);
                self.contacts_dirty = true;
                true
            }
            None => false,
        }
    }

    // --- Add / remove. Add fills the first free bitmap slot with a default
    // record and returns its index; remove clears the slot (serialize then
    // zeros the record and updates the bitmap). ---

    /// Add a channel at the first free slot; returns its index or `None` if full.
    pub fn add_channel(&mut self) -> Option<usize> {
        let i = first_free(&self.channels)?;
        self.channels[i] = Some(Channel::default_record(i));
        Some(i)
    }

    /// Remove channel `index`, scrubbing it from every zone's member list.
    /// Returns false if it was not active.
    pub fn remove_channel(&mut self, index: usize) -> bool {
        if self.channels.get(index).map(|c| c.is_none()).unwrap_or(true) {
            return false;
        }
        self.channels[index] = None;
        let target = index as u16;
        for zone in self.zones.iter_mut().flatten() {
            if zone.channels.contains(&target) {
                let kept: Vec<u16> =
                    zone.channels.iter().copied().filter(|&c| c != target).collect();
                zone.set_members(&kept);
            }
        }
        true
    }

    /// Add a zone at the first free slot; returns its index or `None` if full.
    pub fn add_zone(&mut self) -> Option<usize> {
        let i = first_free(&self.zones)?;
        self.zones[i] = Some(Zone::default_record(i));
        Some(i)
    }

    /// Remove zone `index`. Returns false if it was not active.
    pub fn remove_zone(&mut self, index: usize) -> bool {
        match self.zones.get_mut(index) {
            Some(slot @ Some(_)) => {
                *slot = None;
                true
            }
            _ => false,
        }
    }

    /// Add a contact at the first free slot; returns its index or `None` if full.
    pub fn add_contact(&mut self) -> Option<usize> {
        let i = first_free(&self.contacts)?;
        self.contacts[i] = Some(Contact::default_record(i));
        self.contacts_dirty = true;
        Some(i)
    }

    /// Remove contact `index`, scrubbing it from every group list's members.
    /// Channel `contact_index` fields are left untouched (the operator manages
    /// those). Returns false if it was not active.
    pub fn remove_contact(&mut self, index: usize) -> bool {
        if self.contacts.get(index).map(|c| c.is_none()).unwrap_or(true) {
            return false;
        }
        self.contacts[index] = None;
        self.contacts_dirty = true;
        let target = index as u32;
        for gl in self.group_lists.iter_mut().flatten() {
            if gl.members.contains(&target) {
                let kept: Vec<u32> =
                    gl.members.iter().copied().filter(|&m| m != target).collect();
                gl.set_members(&kept);
            }
        }
        true
    }

    /// Add a group list at the first free slot; returns its index or `None`.
    pub fn add_group_list(&mut self) -> Option<usize> {
        let i = first_free(&self.group_lists)?;
        self.group_lists[i] = Some(GroupList::default_record(i));
        Some(i)
    }

    /// Remove group list `index`. Returns false if it was not active.
    pub fn remove_group_list(&mut self, index: usize) -> bool {
        match self.group_lists.get_mut(index) {
            Some(slot @ Some(_)) => {
                *slot = None;
                true
            }
            _ => false,
        }
    }

    /// Add a radio ID at the first free slot; returns its index or `None`.
    pub fn add_radio_id(&mut self) -> Option<usize> {
        let i = first_free(&self.radio_ids)?;
        self.radio_ids[i] = Some(RadioId::default_record(i));
        Some(i)
    }

    /// Remove radio ID `index`. Returns false if it was not active.
    pub fn remove_radio_id(&mut self, index: usize) -> bool {
        match self.radio_ids.get_mut(index) {
            Some(slot @ Some(_)) => {
                *slot = None;
                true
            }
            _ => false,
        }
    }

    // --- Move: relocate a record to a chosen free slot, remapping everything
    // that referenced it by index. This backs the editor's editable "#" column
    // and lets the operator place a newly added entry at a specific slot. ---

    /// Move channel `from` to the free slot `to`, remapping every zone member
    /// that referenced it. Errors if `from` is inactive, `to` is out of range or
    /// occupied, or `from == to`.
    pub fn move_channel(&mut self, from: usize, to: usize) -> std::result::Result<(), String> {
        relocate(&mut self.channels, from, to, "channel", |r, i| r.index = i)?;
        let (a, b) = (from as u16, to as u16);
        for zone in self.zones.iter_mut().flatten() {
            if zone.channels.contains(&a) {
                let remapped: Vec<u16> =
                    zone.channels.iter().map(|&c| if c == a { b } else { c }).collect();
                zone.set_members(&remapped);
            }
        }
        Ok(())
    }

    /// Move zone `from` to the free slot `to`. Zones are referenced nowhere by
    /// index, so only the slot changes.
    pub fn move_zone(&mut self, from: usize, to: usize) -> std::result::Result<(), String> {
        relocate(&mut self.zones, from, to, "zone", |r, i| r.index = i)
    }

    /// Move contact `from` to the free slot `to`, remapping every channel's
    /// `contact_index` and every group-list member that referenced it.
    pub fn move_contact(&mut self, from: usize, to: usize) -> std::result::Result<(), String> {
        relocate(&mut self.contacts, from, to, "contact", |r, i| r.index = i)?;
        self.contacts_dirty = true;
        let (a, b) = (from as u32, to as u32);
        for ch in self.channels.iter_mut().flatten() {
            if ch.contact_index == a {
                ch.set_contact_index(b);
            }
        }
        for gl in self.group_lists.iter_mut().flatten() {
            if gl.members.contains(&a) {
                let remapped: Vec<u32> =
                    gl.members.iter().map(|&m| if m == a { b } else { m }).collect();
                gl.set_members(&remapped);
            }
        }
        Ok(())
    }

    /// Move group list `from` to the free slot `to`, remapping every channel's
    /// `group_list_index` (the `0xff` = none sentinel never matches a real slot).
    pub fn move_group_list(&mut self, from: usize, to: usize) -> std::result::Result<(), String> {
        relocate(&mut self.group_lists, from, to, "group list", |r, i| r.index = i)?;
        let (a, b) = (from as u8, to as u8);
        for ch in self.channels.iter_mut().flatten() {
            if ch.group_list_index == a {
                ch.set_group_list_index(b);
            }
        }
        Ok(())
    }

    /// Move radio ID `from` to the free slot `to`, remapping every channel's
    /// `radio_id_index` that referenced it.
    pub fn move_radio_id(&mut self, from: usize, to: usize) -> std::result::Result<(), String> {
        relocate(&mut self.radio_ids, from, to, "radio ID", |r, i| r.index = i)?;
        let (a, b) = (from as u8, to as u8);
        for ch in self.channels.iter_mut().flatten() {
            if ch.radio_id_index == a {
                ch.set_radio_id_index(b);
            }
        }
        Ok(())
    }

    /// Build the serializable projection consumed by the CLI `dump` command.
    pub fn to_json(&self) -> CodeplugJson<'_> {
        CodeplugJson {
            channels: self.channels().collect(),
            zones: self.zones().collect(),
            contacts: self.contacts().collect(),
            group_lists: self.group_lists().collect(),
            radio_ids: self.radio_ids().collect(),
        }
    }
}

/// Index of the first `None` slot in a sparse record vector, or `None` if full.
fn first_free<T>(slots: &[Option<T>]) -> Option<usize> {
    slots.iter().position(|s| s.is_none())
}

/// Relocate the active record at `from` to the free slot `to` within a sparse
/// slot vector, running `set_index` to update the record's own index field.
/// Errors (leaving the vector untouched) if `from == to`, `to` is out of range
/// or occupied, or `from` is inactive.
fn relocate<T>(
    slots: &mut [Option<T>],
    from: usize,
    to: usize,
    kind: &str,
    set_index: impl FnOnce(&mut T, usize),
) -> std::result::Result<(), String> {
    if from == to {
        return Err(format!("{kind} {from} is already at slot {to}"));
    }
    if to >= slots.len() {
        return Err(format!("{kind} slot {to} is out of range (max {})", slots.len() - 1));
    }
    if slots.get(from).map(|s| s.is_none()).unwrap_or(true) {
        return Err(format!("{kind} {from} is not active"));
    }
    if slots[to].is_some() {
        return Err(format!("{kind} slot {to} is already occupied"));
    }
    let mut rec = slots[from].take().unwrap();
    set_index(&mut rec, to);
    slots[to] = Some(rec);
    Ok(())
}

/// Radio address of channel `i`: `banks + (i/128)*bankStride + (i%128)*64`.
pub fn channel_addr(i: usize) -> u32 {
    let bank = (i / CHANNELS_PER_BANK) as u32;
    let idx = (i % CHANNELS_PER_BANK) as u32;
    CHANNEL_BANKS + bank * BETWEEN_CHANNEL_BANKS + idx * CHANNEL_SIZE as u32
}

/// Radio address of zone `i`'s name slot.
pub fn zone_name_addr(i: usize) -> u32 {
    ZONE_NAMES + (i as u32) * BETWEEN_ZONE_NAMES
}

/// Radio address of zone `i`'s channel list.
pub fn zone_channels_addr(i: usize) -> u32 {
    ZONE_CHANNELS + (i as u32) * BETWEEN_ZONE_CHANNELS
}

/// Radio address of contact `i`: `banks + (i/1000)*bankStride + (i%1000)*100`
/// (qdmr `encodeContacts` packs contacts contiguously within each bank).
pub fn contact_addr(i: usize) -> u32 {
    let bank = (i / CONTACTS_PER_BANK) as u32;
    let idx = (i % CONTACTS_PER_BANK) as u32;
    CONTACT_BANKS + bank * BETWEEN_CONTACT_BANKS + idx * CONTACT_SIZE as u32
}

/// Radio address of the [`CONTACT_BLOCK`]-sized storage block (4 contacts) that
/// contains contact `i`. Reads/writes over the wire move whole blocks, matching
/// the radio's allocation granularity (qdmr `allocateContacts`).
pub fn contact_block_addr(i: usize) -> u32 {
    let bank = (i / CONTACTS_PER_BANK) as u32;
    let local_block = ((i % CONTACTS_PER_BANK) / CONTACTS_PER_BLOCK) as u32;
    CONTACT_BANKS + bank * BETWEEN_CONTACT_BANKS + local_block * CONTACT_BLOCK as u32
}

/// Radio address of group list `i`'s slot.
pub fn group_list_addr(i: usize) -> u32 {
    GROUP_LISTS + (i as u32) * BETWEEN_GROUP_LISTS
}

/// Radio address of radio-ID `i`'s record.
pub fn radio_id_addr(i: usize) -> u32 {
    RADIO_IDS + (i as u32) * RADIO_ID_SIZE as u32
}

/// Map a radio memory address to its offset within the concatenated codeplug
/// image, or `None` if the address falls outside every region in [`REGIONS`].
pub fn global_offset(addr: u32) -> Option<usize> {
    let mut base = 0usize;
    for r in REGIONS {
        let start = r.address;
        let end = r.address as usize + r.length;
        if addr >= start && (addr as usize) < end {
            return Some(base + (addr - start) as usize);
        }
        base += r.length;
    }
    None
}

/// Borrow `len` bytes of a region at radio address `addr` from the image.
fn read_region(raw: &[u8], addr: u32, len: usize) -> Result<&[u8]> {
    let off =
        global_offset(addr).ok_or_else(|| Error::Parse(format!("address 0x{addr:08X} unmapped")))?;
    raw.get(off..off + len)
        .ok_or_else(|| Error::Parse(format!("region at 0x{addr:08X} truncated")))
}

/// Test whether bit `i` is set in a little-endian bit-per-item bitmap
/// (`data[i/8] & (1 << (i%8))`).
fn bit_set(bitmap: &[u8], i: usize) -> bool {
    let byte = i / 8;
    let bit = i % 8;
    bitmap.get(byte).map(|b| (b >> bit) & 1 == 1).unwrap_or(false)
}

/// Test whether bit `i` is *clear* — the "active" test for an inverted bitmap
/// (qdmr `InvertedBitmapElement`, used by the contact bitmap).
fn bit_clear(bitmap: &[u8], i: usize) -> bool {
    !bit_set(bitmap, i)
}

/// Set or clear bit `i` of the bitmap region at radio address `bitmap_addr`
/// within the serialized image `out`, honoring the bitmap's polarity. For a
/// normal bitmap `active` sets the bit; for an `inverted` bitmap `active`
/// clears it. Only the single target bit is touched, so trailing/padding bits
/// are preserved. A no-op if the address is unmapped.
fn write_bit(out: &mut [u8], bitmap_addr: u32, i: usize, active: bool, inverted: bool) {
    let Some(base) = global_offset(bitmap_addr) else {
        return;
    };
    let byte = base + i / 8;
    let mask = 1u8 << (i % 8);
    // Normal: active => set. Inverted: active => clear.
    if active != inverted {
        out[byte] |= mask;
    } else {
        out[byte] &= !mask;
    }
}

/// Round `n` up to the next multiple of `a` (qdmr `align_size`).
fn align_up(n: usize, a: usize) -> usize {
    n.div_ceil(a) * a
}

/// Encode a DMR ID as 8 little-endian BCD digits packed into a `u32` (qdmr
/// `encode_dmr_id_bcd_le`): the least-significant byte holds the two lowest
/// digits. Used to build contact ID-table entries.
fn encode_dmr_id_bcd_le(no: u32) -> u32 {
    let b0 = (((no / 10) % 10) << 4) | (no % 10);
    let b1 = (((no / 1000) % 10) << 4) | ((no / 100) % 10);
    let b2 = (((no / 100000) % 10) << 4) | ((no / 10000) % 10);
    let b3 = ((no / 10000000) << 4) | ((no / 1000000) % 10);
    u32::from_le_bytes([b0 as u8, b1 as u8, b2 as u8, b3 as u8])
}

/// Decode 8 big-endian BCD digits (4 bytes, most-significant nibble first).
pub(crate) fn get_bcd8_be(bytes: &[u8]) -> u32 {
    let mut v = 0u32;
    for &b in &bytes[..4] {
        v = v * 100 + (b >> 4) as u32 * 10 + (b & 0x0f) as u32;
    }
    v
}

/// Encode `val` as 8 big-endian BCD digits into `out[..4]` (most-significant
/// nibble first). Digits above 8 are dropped (`val` is taken mod 10^8).
pub(crate) fn set_bcd8_be(out: &mut [u8], mut val: u32) {
    for i in (0..4).rev() {
        let pair = (val % 100) as u8;
        out[i] = ((pair / 10) << 4) | (pair % 10);
        val /= 100;
    }
}

/// Read a fixed-length Latin1 string, stopping at the first `fill` byte, and
/// trim trailing spaces for display.
pub(crate) fn read_ascii(bytes: &[u8], fill: u8) -> String {
    let end = bytes.iter().position(|&b| b == fill).unwrap_or(bytes.len());
    let s: String = bytes[..end].iter().map(|&b| b as char).collect();
    s.trim_end_matches(' ').to_string()
}

/// Write a fixed-length Latin1 string into `dst`, padding/terminating the
/// remainder with `fill`. `s` is truncated to `dst.len()` bytes.
pub(crate) fn write_ascii(dst: &mut [u8], s: &str, fill: u8) {
    for b in dst.iter_mut() {
        *b = fill;
    }
    for (slot, byte) in dst.iter_mut().zip(s.bytes()) {
        *slot = byte;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic codeplug image by hand-writing known raw bytes — no
    /// real-radio data, and deliberately not via the encoders under test, so a
    /// `parse` → `serialize` identity is a genuine check of the codec rather
    /// than a tautology. Sets channels 0 and 1 and zone 0.
    fn synthetic_codeplug() -> Vec<u8> {
        let mut raw = vec![0u8; crate::device::codeplug_size()];

        // The contact bitmap is inverted (active = bit clear), so a truly empty
        // codeplug has it all-set. Match that or every zeroed slot reads active.
        let ct_bm = global_offset(CONTACT_BITMAP).unwrap();
        for b in raw[ct_bm..ct_bm + 0x500].iter_mut() {
            *b = 0xff;
        }

        // --- Channel valid-bitmap: mark channels 0 and 1. ---
        let cbm = global_offset(CHANNEL_BITMAP).unwrap();
        raw[cbm] |= 0b0000_0011;

        // --- Channel 0: 439.0000 MHz, +5.0 MHz offset, digital, TS2, CC1. ---
        write_channel_raw(
            &mut raw,
            0,
            439_000_000,
            5_000_000,
            /*mode=*/ 1,
            /*repeater=*/ 1,
            /*color=*/ 1,
            /*ts_bit=*/ 1,
            "CALLING",
        );
        // --- Channel 1: 146.5200 MHz simplex, analog, TS1, CC0. ---
        write_channel_raw(
            &mut raw,
            1,
            146_520_000,
            0,
            /*mode=*/ 0,
            /*repeater=*/ 0,
            /*color=*/ 0,
            /*ts_bit=*/ 0,
            "SIMPLEX 2M",
        );

        // --- Zone valid-bitmap: mark zone 0. ---
        let zbm = global_offset(ZONE_BITMAP).unwrap();
        raw[zbm] |= 0b0000_0001;

        // --- Zone 0: name "Zone A", members [0, 1]. ---
        let name_off = global_offset(zone_name_addr(0)).unwrap();
        write_ascii(&mut raw[name_off..name_off + 16], "Zone A", 0x00);
        let ch_off = global_offset(zone_channels_addr(0)).unwrap();
        for slot in raw[ch_off..ch_off + ZONE_CHANNELS_SLOT].iter_mut() {
            *slot = 0xff;
        }
        raw[ch_off..ch_off + 2].copy_from_slice(&0u16.to_le_bytes());
        raw[ch_off + 2..ch_off + 4].copy_from_slice(&1u16.to_le_bytes());

        raw
    }

    /// Hand-write one channel's 64-byte record at channel index `i`.
    #[allow(clippy::too_many_arguments)]
    fn write_channel_raw(
        raw: &mut [u8],
        i: usize,
        rx_hz: u32,
        tx_off_hz: u32,
        mode: u8,
        repeater: u8,
        color: u8,
        ts_bit: u8,
        name: &str,
    ) {
        let off = global_offset(channel_addr(i)).unwrap();
        let rec = &mut raw[off..off + CHANNEL_SIZE];
        set_bcd8_be(&mut rec[0..4], rx_hz / 10);
        set_bcd8_be(&mut rec[4..8], tx_off_hz / 10);
        // 0x08: mode (bits 0-1), power (2-3), bandwidth (4-5), repeater (6-7).
        rec[0x08] = (mode & 0b11) | ((repeater & 0b11) << 6);
        rec[0x20] = color;
        rec[0x21] = ts_bit & 0b1;
        write_ascii(&mut rec[0x23..0x23 + 16], name, 0x00);
    }

    #[test]
    fn parse_reads_channels_and_zones() {
        let raw = synthetic_codeplug();
        let cp = Codeplug::parse(&raw).unwrap();

        let chans: Vec<&Channel> = cp.channels().collect();
        assert_eq!(chans.len(), 2);
        assert_eq!(chans[0].name, "CALLING");
        assert_eq!(chans[0].rx_frequency_hz, 439_000_000);
        assert_eq!(chans[0].tx_frequency_hz, 444_000_000);
        assert_eq!(chans[0].mode, ChannelMode::Digital);
        assert_eq!(chans[0].color_code, 1);
        assert_eq!(chans[0].time_slot, 2);
        assert_eq!(chans[1].name, "SIMPLEX 2M");
        assert_eq!(chans[1].rx_frequency_hz, 146_520_000);
        assert_eq!(chans[1].tx_frequency_hz, 146_520_000);
        assert_eq!(chans[1].mode, ChannelMode::Analog);

        let zones: Vec<&Zone> = cp.zones().collect();
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].name, "Zone A");
        assert_eq!(zones[0].channels, vec![0, 1]);
    }

    #[test]
    fn roundtrip_is_byte_identical() {
        let raw = synthetic_codeplug();
        let cp = Codeplug::parse(&raw).unwrap();
        let out = cp.serialize();
        assert_eq!(out, raw, "parse -> serialize must be byte-identical");
    }

    #[test]
    fn editing_name_changes_only_name_bytes() {
        let raw = synthetic_codeplug();
        let mut cp = Codeplug::parse(&raw).unwrap();
        cp.channel_mut(0).unwrap().set_name("REPEATER");
        let out = cp.serialize();

        let rec = global_offset(channel_addr(0)).unwrap();
        let name_lo = rec + 0x23;
        let name_hi = name_lo + 16;
        for (i, (a, b)) in raw.iter().zip(out.iter()).enumerate() {
            if (name_lo..name_hi).contains(&i) {
                continue;
            }
            assert_eq!(a, b, "byte {i} changed but only the name field should");
        }
        assert_ne!(&raw[name_lo..name_hi], &out[name_lo..name_hi]);
        assert_eq!(Codeplug::parse(&out).unwrap().channels().next().unwrap().name, "REPEATER");
    }

    #[test]
    fn editing_frequency_changes_only_freq_bytes() {
        let raw = synthetic_codeplug();
        let mut cp = Codeplug::parse(&raw).unwrap();
        cp.channel_mut(1).unwrap().set_rx_frequency(147_000_000);
        let out = cp.serialize();

        let rec = global_offset(channel_addr(1)).unwrap();
        for (i, (a, b)) in raw.iter().zip(out.iter()).enumerate() {
            if (rec..rec + 4).contains(&i) {
                continue;
            }
            assert_eq!(a, b, "byte {i} changed but only rx frequency should");
        }
        assert_eq!(
            Codeplug::parse(&out)
                .unwrap()
                .channels()
                .nth(1)
                .unwrap()
                .rx_frequency_hz,
            147_000_000
        );
    }

    #[test]
    fn parse_rejects_wrong_size() {
        let err = Codeplug::parse(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, Error::Parse(_)));
    }

    #[test]
    fn bcd_roundtrip() {
        let mut buf = [0u8; 4];
        set_bcd8_be(&mut buf, 43_900_000);
        assert_eq!(buf, [0x43, 0x90, 0x00, 0x00]);
        assert_eq!(get_bcd8_be(&buf), 43_900_000);
    }

    #[test]
    fn empty_codeplug_has_no_dmr_entities() {
        // The synthetic image marks no contact/group/radio-ID bits, so despite
        // the inverted contact bitmap nothing should read as active.
        let cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        assert_eq!(cp.contacts().count(), 0);
        assert_eq!(cp.group_lists().count(), 0);
        assert_eq!(cp.radio_ids().count(), 0);
    }

    #[test]
    fn channel_setters_roundtrip_through_serialize() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        let ch = cp.channel_mut(1).unwrap();
        ch.set_mode(ChannelMode::Digital);
        ch.set_power(Power::Turbo);
        ch.set_bandwidth(Bandwidth::Wide);
        ch.set_color_code(11);
        ch.set_time_slot(2);
        ch.set_contact_index(7);
        ch.set_radio_id_index(3);
        ch.set_group_list_index(5);
        ch.set_tx_frequency(147_000_000);

        let back = Codeplug::parse(&cp.serialize()).unwrap();
        let ch = back.channels().find(|c| c.index == 1).unwrap();
        assert_eq!(ch.mode, ChannelMode::Digital);
        assert_eq!(ch.power, Power::Turbo);
        assert_eq!(ch.bandwidth, Bandwidth::Wide);
        assert_eq!(ch.color_code, 11);
        assert_eq!(ch.time_slot, 2);
        assert_eq!(ch.contact_index, 7);
        assert_eq!(ch.radio_id_index, 3);
        assert_eq!(ch.group_list_index, 5);
        assert_eq!(ch.tx_frequency_hz, 147_000_000);
    }

    #[test]
    fn add_and_remove_channel_flips_only_its_bitmap_bit() {
        let base = synthetic_codeplug();
        let mut cp = Codeplug::parse(&base).unwrap();
        let idx = cp.add_channel().unwrap();
        assert_eq!(idx, 2, "channels 0,1 taken so next free is 2");
        let with_add = cp.serialize();

        // Exactly bit 2 of the channel bitmap flips, plus channel 2's record.
        let bm = global_offset(CHANNEL_BITMAP).unwrap();
        assert_eq!(base[bm], 0b0000_0011);
        assert_eq!(with_add[bm], 0b0000_0111);

        // Removing it returns to the original active set and byte image.
        let mut cp2 = Codeplug::parse(&with_add).unwrap();
        assert!(cp2.remove_channel(2));
        let back = Codeplug::parse(&cp2.serialize()).unwrap();
        assert_eq!(back.channels().count(), 2);
    }

    #[test]
    fn removing_channel_scrubs_it_from_zones() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        assert_eq!(cp.zones().next().unwrap().channels, vec![0, 1]);
        assert!(cp.remove_channel(1));
        let back = Codeplug::parse(&cp.serialize()).unwrap();
        assert_eq!(back.zones().next().unwrap().channels, vec![0]);
    }

    #[test]
    fn add_contact_talkgroup_roundtrips_and_rebuilds_id_table() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        let i = cp.add_contact().unwrap();
        assert_eq!(i, 0);
        assert!(cp.set_contact_name(i, "PARROT"));
        assert!(cp.set_contact_number(i, 9990));
        assert!(cp.set_contact_call_type(i, CallType::Private));

        let out = cp.serialize();
        let back = Codeplug::parse(&out).unwrap();
        let c = back.contacts().next().unwrap();
        assert_eq!(c.name, "PARROT");
        assert_eq!(c.number, 9990);
        assert_eq!(c.call_type, CallType::Private);

        // The ID-table entry: id = (bcd_le(9990) << 1) | group_flag(0), index 0.
        let base = global_offset(CONTACT_ID_TABLE).unwrap();
        let id = u32::from_le_bytes([out[base], out[base + 1], out[base + 2], out[base + 3]]);
        let index = u32::from_le_bytes([out[base + 4], out[base + 5], out[base + 6], out[base + 7]]);
        assert_eq!(id, encode_dmr_id_bcd_le(9990) << 1);
        assert_eq!(index, 0);
    }

    #[test]
    fn group_list_members_roundtrip_and_scrub_on_contact_removal() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        let c0 = cp.add_contact().unwrap();
        let c1 = cp.add_contact().unwrap();
        let gl = cp.add_group_list().unwrap();
        cp.group_list_mut(gl)
            .unwrap()
            .set_members(&[c0 as u32, c1 as u32]);

        let back = Codeplug::parse(&cp.serialize()).unwrap();
        assert_eq!(back.group_lists().next().unwrap().members, vec![0, 1]);

        // Removing contact 0 scrubs it from the group list's members.
        let mut cp2 = back;
        assert!(cp2.remove_contact(0));
        let back2 = Codeplug::parse(&cp2.serialize()).unwrap();
        assert_eq!(back2.group_lists().next().unwrap().members, vec![1]);
    }

    #[test]
    fn add_radio_id_roundtrips() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        let i = cp.add_radio_id().unwrap();
        cp.radio_id_mut(i).unwrap().set_name("HOME");
        cp.radio_id_mut(i).unwrap().set_number(3141592);
        let back = Codeplug::parse(&cp.serialize()).unwrap();
        let rid = back.radio_ids().next().unwrap();
        assert_eq!(rid.name, "HOME");
        assert_eq!(rid.number, 3141592);
    }

    #[test]
    fn unrelated_edit_leaves_contact_tables_untouched() {
        // Populate a codeplug with a contact + its rebuilt tables, then reparse
        // and make a channel-only edit: the contact index / ID table regions
        // must be byte-identical (rebuild is gated on contact changes).
        let mut seed = Codeplug::parse(&synthetic_codeplug()).unwrap();
        seed.add_contact();
        seed.set_contact_number(0, 1234567);
        let with_contact = seed.serialize();

        let mut cp = Codeplug::parse(&with_contact).unwrap();
        cp.channel_mut(0).unwrap().set_name("EDITED");
        let out = cp.serialize();

        for region in [CONTACT_INDEX, CONTACT_ID_TABLE] {
            let base = global_offset(region).unwrap();
            let len = if region == CONTACT_INDEX {
                NUM_CONTACTS * 4
            } else {
                NUM_CONTACTS * CONTACT_MAP_ENTRY
            };
            assert_eq!(
                with_contact[base..base + len],
                out[base..base + len],
                "contact lookup region 0x{region:08x} changed on an unrelated edit"
            );
        }
    }

    #[test]
    fn move_channel_relocates_and_remaps_zone_members() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        // Zone 0 = [0, 1]; move channel 1 to the free slot 5.
        cp.move_channel(1, 5).unwrap();
        let back = Codeplug::parse(&cp.serialize()).unwrap();

        assert!(back.channels().any(|c| c.index == 5 && c.name == "SIMPLEX 2M"));
        assert!(!back.channels().any(|c| c.index == 1));
        assert_eq!(back.zones().next().unwrap().channels, vec![0, 5]);
    }

    #[test]
    fn move_contact_remaps_channel_and_group_list_references() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        cp.add_contact(); // slot 0
        cp.set_contact_number(0, 1234567);
        cp.channel_mut(0).unwrap().set_contact_index(0);
        let gi = cp.add_group_list().unwrap(); // slot 0
        cp.group_list_mut(gi).unwrap().set_members(&[0]);

        cp.move_contact(0, 3).unwrap();
        let back = Codeplug::parse(&cp.serialize()).unwrap();

        assert!(back.contacts().any(|c| c.index == 3 && c.number == 1234567));
        assert!(!back.contacts().any(|c| c.index == 0));
        assert_eq!(back.channels().find(|c| c.index == 0).unwrap().contact_index, 3);
        assert_eq!(back.group_lists().next().unwrap().members, vec![3]);
    }

    #[test]
    fn move_group_list_leaves_none_sentinel_untouched() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        let gi = cp.add_group_list().unwrap(); // slot 0
        cp.group_list_mut(gi).unwrap().set_name("LOCAL");
        cp.channel_mut(0).unwrap().set_group_list_index(0); // references it
        cp.channel_mut(1).unwrap().set_group_list_index(0xff); // none

        cp.move_group_list(0, 4).unwrap();
        let back = Codeplug::parse(&cp.serialize()).unwrap();

        assert!(back.group_lists().any(|g| g.index == 4 && g.name == "LOCAL"));
        assert_eq!(back.channels().find(|c| c.index == 0).unwrap().group_list_index, 4);
        assert_eq!(back.channels().find(|c| c.index == 1).unwrap().group_list_index, 0xff);
    }

    #[test]
    fn move_rejects_occupied_and_out_of_range() {
        let mut cp = Codeplug::parse(&synthetic_codeplug()).unwrap();
        // Slot 0 is occupied (channel 0 active).
        assert!(cp.move_channel(1, 0).is_err());
        // Out of range.
        assert!(cp.move_channel(1, NUM_CHANNELS).is_err());
        // Both failures leave channel 1 where it was.
        assert!(cp.channels().any(|c| c.index == 1));
    }
}
