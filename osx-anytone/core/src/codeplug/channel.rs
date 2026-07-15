//! The 64-byte D878UVII channel record.
//!
//! Offsets are from qdmr `lib/anytone_codeplug.hh`
//! `AnytoneCodeplug::ChannelElement::Offset` (the D878UV element extends this
//! base and keeps the layout; `size()` = 0x40). See `mod.rs` for the full
//! offset/source table. The struct keeps the raw 64 bytes as the source of
//! truth so [`encode`](Channel::encode) is lossless; the decoded fields are a
//! cache refreshed on every edit.

use serde::Serialize;

use super::{get_bcd8_be, read_ascii, set_bcd8_be, write_ascii};
use crate::error::{Error, Result};

/// Size of a channel record in bytes (qdmr `ChannelElement::size()` = 0x40).
pub const CHANNEL_SIZE: usize = 64;

/// Offset of the 16-byte channel name (qdmr `Offset::name()`).
const NAME_OFFSET: usize = 0x23;
/// Maximum channel name length (qdmr `Limit::nameLength()`).
const NAME_LEN: usize = 16;
/// Offset of the packed mode/power/bandwidth/repeater flags byte
/// (qdmr `Offset::channelMode()` etc., all within byte 0x08).
const FLAGS_OFFSET: usize = 0x08;
/// Offset of the digital-contact index, u32 little-endian
/// (qdmr `Offset::contactIndex()`).
const CONTACT_INDEX_OFFSET: usize = 0x14;
/// Offset of the radio-ID index, u8 (qdmr `Offset::radioIdIndex()`).
const RADIO_ID_INDEX_OFFSET: usize = 0x18;
/// Offset of the RX group-list index, u8 (qdmr `Offset::groupListIndex()`).
const GROUP_LIST_INDEX_OFFSET: usize = 0x1c;
/// Offset of the DMR color code, u8 (qdmr `Offset::colorCode()`).
const COLOR_CODE_OFFSET: usize = 0x20;
/// Offset of the byte holding the time-slot bit (qdmr `Offset::timeSlot()`).
const TIME_SLOT_OFFSET: usize = 0x21;

/// Channel operating mode (qdmr `ChannelElement::Mode`, 2-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ChannelMode {
    /// Analog FM.
    Analog,
    /// Digital DMR.
    Digital,
    /// Mixed: analog TX, digital-capable RX.
    MixedAnalog,
    /// Mixed: digital TX, analog-capable RX.
    MixedDigital,
}

impl ChannelMode {
    /// Decode the 2-bit mode field.
    fn from_bits(v: u8) -> ChannelMode {
        match v & 0b11 {
            0 => ChannelMode::Analog,
            1 => ChannelMode::Digital,
            2 => ChannelMode::MixedAnalog,
            _ => ChannelMode::MixedDigital,
        }
    }

    /// Encode as the 2-bit mode field.
    fn to_bits(self) -> u8 {
        match self {
            ChannelMode::Analog => 0,
            ChannelMode::Digital => 1,
            ChannelMode::MixedAnalog => 2,
            ChannelMode::MixedDigital => 3,
        }
    }
}

/// Transmit power level (qdmr `ChannelElement::Power`, 2-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Power {
    /// Low power.
    Low,
    /// Medium power.
    Mid,
    /// High power.
    High,
    /// Turbo/maximum power.
    Turbo,
}

impl Power {
    /// Decode the 2-bit power field.
    fn from_bits(v: u8) -> Power {
        match v & 0b11 {
            0 => Power::Low,
            1 => Power::Mid,
            2 => Power::High,
            _ => Power::Turbo,
        }
    }

    /// Encode as the 2-bit power field.
    fn to_bits(self) -> u8 {
        match self {
            Power::Low => 0,
            Power::Mid => 1,
            Power::High => 2,
            Power::Turbo => 3,
        }
    }
}

/// Channel bandwidth (qdmr `ChannelElement`, bit 4 of byte 0x08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Bandwidth {
    /// 12.5 kHz narrow.
    Narrow,
    /// 25 kHz wide.
    Wide,
}

impl Bandwidth {
    /// Decode the 2-bit bandwidth field (0 = narrow, otherwise wide).
    fn from_bits(v: u8) -> Bandwidth {
        if v & 0b11 == 0 {
            Bandwidth::Narrow
        } else {
            Bandwidth::Wide
        }
    }

    /// Encode as the 2-bit bandwidth field (narrow = 0, wide = 1).
    fn to_bits(self) -> u8 {
        match self {
            Bandwidth::Narrow => 0,
            Bandwidth::Wide => 1,
        }
    }
}

/// Repeater/offset mode (qdmr `ChannelElement::RepeaterMode`, bits 6-7 of
/// byte 0x08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RepeaterMode {
    /// Simplex: TX = RX, the offset is ignored.
    Simplex,
    /// Repeater with a positive TX offset.
    Positive,
    /// Repeater with a negative TX offset.
    Negative,
}

impl RepeaterMode {
    /// Decode the 2-bit repeater-mode field.
    fn from_bits(v: u8) -> RepeaterMode {
        match v & 0b11 {
            1 => RepeaterMode::Positive,
            2 => RepeaterMode::Negative,
            _ => RepeaterMode::Simplex,
        }
    }

    /// Encode as the 2-bit repeater-mode field.
    fn to_bits(self) -> u8 {
        match self {
            RepeaterMode::Simplex => 0,
            RepeaterMode::Positive => 1,
            RepeaterMode::Negative => 2,
        }
    }
}

/// A single channel. Decoded fields are for inspection/JSON; `raw` is the
/// authoritative image that [`encode`](Channel::encode) returns unchanged
/// unless a setter has edited it.
#[derive(Debug, Clone, Serialize)]
pub struct Channel {
    /// Channel index (position in the bitmap / bank layout).
    pub index: usize,
    /// Channel name.
    pub name: String,
    /// Receive frequency in Hz.
    pub rx_frequency_hz: u32,
    /// Transmit-offset magnitude in Hz (as stored).
    pub tx_offset_hz: u32,
    /// Effective transmit frequency in Hz, derived from RX + repeater mode.
    pub tx_frequency_hz: u32,
    /// Repeater/offset mode.
    pub repeater_mode: RepeaterMode,
    /// Operating mode (analog/digital/mixed).
    pub mode: ChannelMode,
    /// Transmit power level.
    pub power: Power,
    /// Channel bandwidth.
    pub bandwidth: Bandwidth,
    /// DMR color code (0-15).
    pub color_code: u8,
    /// DMR time slot (1 or 2).
    pub time_slot: u8,
    /// Index into the digital contact table.
    pub contact_index: u32,
    /// Index into the radio-ID table.
    pub radio_id_index: u8,
    /// Index into the RX group-list table (`0xff` = none).
    pub group_list_index: u8,
    /// Authoritative raw record; skipped in JSON output.
    #[serde(skip)]
    raw: [u8; CHANNEL_SIZE],
}

impl Channel {
    /// Parse a 64-byte record at channel `index`. Returns [`Error::Parse`] if
    /// the slice is not exactly [`CHANNEL_SIZE`] bytes.
    pub fn parse(index: usize, rec: &[u8]) -> Result<Channel> {
        if rec.len() != CHANNEL_SIZE {
            return Err(Error::Parse(format!(
                "channel record is {} bytes, expected {CHANNEL_SIZE}",
                rec.len()
            )));
        }
        let mut raw = [0u8; CHANNEL_SIZE];
        raw.copy_from_slice(rec);
        Ok(Self::from_raw(index, raw))
    }

    /// The raw 64-byte record for serialization (lossless).
    pub fn encode(&self) -> &[u8; CHANNEL_SIZE] {
        &self.raw
    }

    /// Set the channel name, rewriting only the 16-byte name field in `raw`.
    pub fn set_name(&mut self, name: &str) {
        write_ascii(&mut self.raw[NAME_OFFSET..NAME_OFFSET + NAME_LEN], name, 0x00);
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the receive frequency in Hz, rewriting only the 4 BCD bytes at
    /// offset 0. The value is stored in units of 10 Hz.
    pub fn set_rx_frequency(&mut self, hz: u32) {
        set_bcd8_be(&mut self.raw[0..4], hz / 10);
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the effective transmit frequency in Hz. The radio stores TX as a
    /// repeater mode (bits 6-7 of the flags byte) plus a BCD offset magnitude
    /// at 0x04, both relative to RX; this rewrites exactly those bytes. Equal
    /// to RX means simplex; higher/lower selects the positive/negative offset.
    pub fn set_tx_frequency(&mut self, hz: u32) {
        let rx = get_bcd8_be(&self.raw[0..4]) * 10;
        let (mode, offset_hz) = match hz.cmp(&rx) {
            std::cmp::Ordering::Equal => (RepeaterMode::Simplex, 0),
            std::cmp::Ordering::Greater => (RepeaterMode::Positive, hz - rx),
            std::cmp::Ordering::Less => (RepeaterMode::Negative, rx - hz),
        };
        set_bcd8_be(&mut self.raw[4..8], offset_hz / 10);
        self.set_flag_field(6, mode.to_bits());
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the operating mode (analog/digital/mixed), bits 0-1 of the flags byte.
    pub fn set_mode(&mut self, mode: ChannelMode) {
        self.set_flag_field(0, mode.to_bits());
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the transmit power level, bits 2-3 of the flags byte.
    pub fn set_power(&mut self, power: Power) {
        self.set_flag_field(2, power.to_bits());
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the channel bandwidth, bits 4-5 of the flags byte.
    pub fn set_bandwidth(&mut self, bandwidth: Bandwidth) {
        self.set_flag_field(4, bandwidth.to_bits());
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the DMR color code (0-15), clamped, at 0x20.
    pub fn set_color_code(&mut self, cc: u8) {
        self.raw[COLOR_CODE_OFFSET] = cc.min(15);
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the DMR time slot (1 or 2); only bit 0 of byte 0x21 is touched.
    pub fn set_time_slot(&mut self, slot: u8) {
        let bit = if slot >= 2 { 1 } else { 0 };
        self.raw[TIME_SLOT_OFFSET] = (self.raw[TIME_SLOT_OFFSET] & !0b1) | bit;
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the digital-contact index (into the contact table), u32-le at 0x14.
    pub fn set_contact_index(&mut self, index: u32) {
        self.raw[CONTACT_INDEX_OFFSET..CONTACT_INDEX_OFFSET + 4]
            .copy_from_slice(&index.to_le_bytes());
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the radio-ID index (into the radio-ID table), u8 at 0x18.
    pub fn set_radio_id_index(&mut self, index: u8) {
        self.raw[RADIO_ID_INDEX_OFFSET] = index;
        *self = Self::from_raw(self.index, self.raw);
    }

    /// Set the RX group-list index (into the group-list table), u8 at 0x1c.
    /// `0xff` means "none" in the vendor CPS.
    pub fn set_group_list_index(&mut self, index: u8) {
        self.raw[GROUP_LIST_INDEX_OFFSET] = index;
        *self = Self::from_raw(self.index, self.raw);
    }

    /// A blank but valid channel record for `index`: analog, 146.520 MHz
    /// simplex, narrow, low power, TS1/CC0, named "NEW". Callers typically edit
    /// it immediately after adding.
    pub fn default_record(index: usize) -> Channel {
        let mut raw = [0u8; CHANNEL_SIZE];
        set_bcd8_be(&mut raw[0..4], 146_520_000 / 10);
        write_ascii(&mut raw[NAME_OFFSET..NAME_OFFSET + NAME_LEN], "NEW", 0x00);
        Self::from_raw(index, raw)
    }

    /// Overwrite a 2-bit field within the flags byte at `shift` (0/2/4/6),
    /// preserving the other three fields.
    fn set_flag_field(&mut self, shift: u8, value: u8) {
        let mask = 0b11u8 << shift;
        self.raw[FLAGS_OFFSET] = (self.raw[FLAGS_OFFSET] & !mask) | ((value & 0b11) << shift);
    }

    /// Rebuild the decoded fields from `raw`.
    fn from_raw(index: usize, raw: [u8; CHANNEL_SIZE]) -> Channel {
        let rx_frequency_hz = get_bcd8_be(&raw[0..4]) * 10;
        let tx_offset_hz = get_bcd8_be(&raw[4..8]) * 10;
        let flags = raw[0x08];
        let repeater_mode = RepeaterMode::from_bits(flags >> 6);
        let tx_frequency_hz = match repeater_mode {
            RepeaterMode::Simplex => rx_frequency_hz,
            RepeaterMode::Positive => rx_frequency_hz.saturating_add(tx_offset_hz),
            RepeaterMode::Negative => rx_frequency_hz.saturating_sub(tx_offset_hz),
        };
        Channel {
            index,
            name: read_ascii(&raw[NAME_OFFSET..NAME_OFFSET + NAME_LEN], 0x00),
            rx_frequency_hz,
            tx_offset_hz,
            tx_frequency_hz,
            repeater_mode,
            mode: ChannelMode::from_bits(flags),
            power: Power::from_bits(flags >> 2),
            bandwidth: Bandwidth::from_bits(flags >> 4),
            color_code: raw[0x20],
            time_slot: 1 + (raw[0x21] & 0b1),
            contact_index: u32::from_le_bytes([raw[0x14], raw[0x15], raw[0x16], raw[0x17]]),
            radio_id_index: raw[RADIO_ID_INDEX_OFFSET],
            group_list_index: raw[GROUP_LIST_INDEX_OFFSET],
            raw,
        }
    }
}
