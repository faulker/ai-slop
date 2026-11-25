//! Sealed telemetry: a heartbeat with everything the board knows about
//! itself, sent over HTTP (`POST /telemetry`) whether or not it has a
//! position. Same layout and key as a report (see [`crate::packet`]), but
//! format byte 2 and nonce domain 1, so it never collides with a report.

use heapless::{String, Vec};
use serde::{Deserialize, Serialize};

use crate::config::Role;
use crate::packet::{self, Header};
use crate::Error;

/// First byte of a telemetry packet.
pub const FORMAT: u8 = 2;
/// Nonce domain (last 4 nonce bytes) for telemetry.
const DOMAIN: u32 = 1;
/// Upper bound on a sealed telemetry packet. HTTP only, so it isn't held to
/// the LoRa-sized `MAX_PACKET`.
pub const MAX_TELEMETRY: usize = 384;
/// Board log lines carried per heartbeat (the newest, if more piled up).
pub const MAX_EVENTS: usize = 6;
/// Longest board log line, matching the board's own log.
pub const EVENT_TEXT: usize = 32;

/// One line from the board's event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardEvent {
    /// Seconds before this heartbeat was built.
    pub age_s: u32,
    pub text: String<EVENT_TEXT>,
}

/// GPS module state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gps {
    /// No NMEA seen: nothing wired up.
    Absent,
    Searching,
    Fix,
}

/// Where the board's best position came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocSource {
    Gps,
    /// City-level, from the network's public IP.
    Ip,
}

/// The board's best position and how old it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub source: LocSource,
    pub lat_e7: i32,
    pub lon_e7: i32,
    pub age_s: u32,
}

/// Everything in one heartbeat. `None` means the board doesn't know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Telemetry {
    pub role: Role,
    /// Heltec hardware revision (2 or 3).
    pub board: u8,
    /// Unix seconds, 0 when the clock isn't set.
    pub ts: u32,
    pub uptime_s: u32,
    pub boot: u16,
    pub batt_mv: Option<u16>,
    pub gps: Gps,
    pub sats: u8,
    pub wifi_rssi: Option<i8>,
    pub lora_tx: u32,
    pub lora_rx: u32,
    pub lora_bad: u32,
    pub lora_rssi: Option<i16>,
    /// Uploads that got a 2xx / that failed, since boot.
    pub sent_ok: u32,
    pub sent_fail: u32,
    /// Reports waiting for an HTTP path.
    pub queued: u16,
    pub position: Option<Position>,
    /// Board log lines since the last heartbeat that got through, oldest first.
    pub events: Vec<BoardEvent, MAX_EVENTS>,
}

/// Encrypts `t` into `out`, returning the packet length.
pub fn seal(key: &[u8; 32], device_id: u32, seq: u32, t: &Telemetry, out: &mut [u8]) -> Result<usize, Error> {
    packet::seal_body(FORMAT, DOMAIN, key, device_id, seq, t, out)
}

/// Reads the plaintext header so the caller can look up the device key.
pub fn peek_header(packet: &[u8]) -> Result<Header, Error> {
    packet::peek_format(FORMAT, packet)
}

/// Verifies and decrypts a telemetry packet.
pub fn open(key: &[u8; 32], packet: &[u8]) -> Result<(Header, Telemetry), Error> {
    packet::open_body::<_, MAX_TELEMETRY>(FORMAT, DOMAIN, key, packet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{Report, MAX_PACKET};

    const KEY: [u8; 32] = [7u8; 32];

    /// Every field at its largest, to check the size bound.
    fn worst() -> Telemetry {
        Telemetry {
            role: Role::Gateway,
            board: u8::MAX,
            ts: u32::MAX,
            uptime_s: u32::MAX,
            boot: u16::MAX,
            batt_mv: Some(u16::MAX),
            gps: Gps::Fix,
            sats: u8::MAX,
            wifi_rssi: Some(i8::MIN),
            lora_tx: u32::MAX,
            lora_rx: u32::MAX,
            lora_bad: u32::MAX,
            lora_rssi: Some(i16::MIN),
            sent_ok: u32::MAX,
            sent_fail: u32::MAX,
            queued: u16::MAX,
            position: Some(Position { source: LocSource::Ip, lat_e7: i32::MIN, lon_e7: i32::MIN, age_s: u32::MAX }),
            events: core::iter::repeat_n(
                BoardEvent { age_s: u32::MAX, text: String::try_from("x".repeat(EVENT_TEXT).as_str()).unwrap() },
                MAX_EVENTS,
            )
            .collect(),
        }
    }

    #[test]
    fn round_trip_and_worst_case_fits() {
        let mut buf = [0u8; MAX_TELEMETRY];
        let n = seal(&KEY, 9, u32::MAX, &worst(), &mut buf).unwrap();
        let (h, t) = open(&KEY, &buf[..n]).unwrap();
        assert_eq!((h.version, h.device_id, h.seq), (FORMAT, 9, u32::MAX));
        assert_eq!(t, worst());
        assert_eq!(peek_header(&buf[..n]).unwrap().device_id, 9);
    }

    #[test]
    fn wrong_key_fails() {
        let mut buf = [0u8; MAX_TELEMETRY];
        let n = seal(&KEY, 9, 1, &worst(), &mut buf).unwrap();
        assert_eq!(open(&[8u8; 32], &buf[..n]), Err(Error::AuthFailed));
    }

    #[test]
    fn reports_and_telemetry_are_not_interchangeable() {
        let mut buf = [0u8; MAX_TELEMETRY];
        let n = seal(&KEY, 9, 1, &worst(), &mut buf).unwrap();
        assert_eq!(packet::open(&KEY, &buf[..n]), Err(Error::BadVersion(FORMAT)));

        let mut rep = [0u8; MAX_PACKET];
        let m = packet::seal(&KEY, &Report { device_id: 9, seq: 1, ..Default::default() }, &mut rep).unwrap();
        assert_eq!(open(&KEY, &rep[..m]), Err(Error::BadVersion(packet::VERSION)));
        // Same seq, but a different nonce domain: forcing the format byte
        // still doesn't verify.
        rep[0] = FORMAT;
        assert_eq!(open(&KEY, &rep[..m]), Err(Error::AuthFailed));
    }
}
