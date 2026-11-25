//! Sealed position reports.
//!
//! Wire layout (all integers little-endian):
//!
//! ```text
//! [version u8][device_id u32][seq u32] | ciphertext(postcard(body)) | tag[16]
//! \________ header, 9 bytes, AAD _____/
//! ```
//!
//! The header stays plaintext so a gateway or server can pick the right key
//! and reject replays before decrypting. It is bound into the tag as AAD.
//! The nonce is `device_id || seq || 0u32`, so `seq` must never repeat for a
//! given key (firmware persists a boot counter in the high bits). Telemetry
//! ([`crate::telemetry`]) uses the same layout with its own format byte and
//! nonce domain.

use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Error;

/// Current wire format version.
pub const VERSION: u8 = 1;
/// Plaintext header length.
pub const HEADER_LEN: usize = 9;
/// Poly1305 tag length.
pub const TAG_LEN: usize = 16;
/// Upper bound on a sealed packet; size LoRa and upload buffers with this.
pub const MAX_PACKET: usize = 64;

/// Report flag: GPS has no current fix; the position is the last known one.
pub const FLAG_NO_FIX: u8 = 1 << 0;
/// Report flag: the tracker considers itself stationary.
pub const FLAG_STATIONARY: u8 = 1 << 1;
/// Report flag: the position is a coarse estimate (IP geolocation, city-level),
/// not a GPS fix.
pub const FLAG_APPROX: u8 = 1 << 2;

/// One position report. Fixed-point fields keep packets small and float-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    pub device_id: u32,
    pub seq: u32,
    /// Unix seconds, 0 when the clock is unknown.
    pub ts: u32,
    /// Degrees * 1e7.
    pub lat_e7: i32,
    /// Degrees * 1e7.
    pub lon_e7: i32,
    /// Meters above mean sea level.
    pub alt_m: i16,
    /// Ground speed in cm/s.
    pub speed_cms: u16,
    /// HDOP * 10, saturating at 255.
    pub hdop_x10: u8,
    /// Battery millivolts, 0 when unknown (vehicle power).
    pub batt_mv: u16,
    /// `FLAG_*` bits.
    pub flags: u8,
}

/// The encrypted part of a report; device_id and seq live in the header.
#[derive(Serialize, Deserialize)]
struct Body {
    ts: u32,
    lat_e7: i32,
    lon_e7: i32,
    alt_m: i16,
    speed_cms: u16,
    hdop_x10: u8,
    batt_mv: u16,
    flags: u8,
}

/// Plaintext header fields, readable without the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub device_id: u32,
    pub seq: u32,
}

/// Builds the 96-bit nonce: `device_id || seq || domain`. Reports use domain
/// 0 and telemetry domain 1, so the two can share one key and seq counters
/// without ever repeating a nonce.
fn nonce(device_id: u32, seq: u32, domain: u32) -> Nonce {
    let mut n = [0u8; 12];
    n[0..4].copy_from_slice(&device_id.to_le_bytes());
    n[4..8].copy_from_slice(&seq.to_le_bytes());
    n[8..12].copy_from_slice(&domain.to_le_bytes());
    Nonce::from(n)
}

/// Seals `body` as `[format][device_id][seq] | ciphertext | tag` into `out`.
/// Shared by reports and [`crate::telemetry`].
pub(crate) fn seal_body<B: Serialize>(
    format: u8,
    domain: u32,
    key: &[u8; 32],
    device_id: u32,
    seq: u32,
    body: &B,
    out: &mut [u8],
) -> Result<usize, Error> {
    if out.len() < HEADER_LEN + TAG_LEN {
        return Err(Error::BufferTooSmall);
    }
    out[0] = format;
    out[1..5].copy_from_slice(&device_id.to_le_bytes());
    out[5..9].copy_from_slice(&seq.to_le_bytes());

    let (header, rest) = out.split_at_mut(HEADER_LEN);
    let body_room = rest.len() - TAG_LEN;
    let body_len = postcard::to_slice(body, &mut rest[..body_room])
        .map_err(|e| match e {
            postcard::Error::SerializeBufferFull => Error::BufferTooSmall,
            _ => Error::Encode,
        })?
        .len();

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let tag = cipher
        .encrypt_in_place_detached(&nonce(device_id, seq, domain), header, &mut rest[..body_len])
        .map_err(|_| Error::Encode)?;
    rest[body_len..body_len + TAG_LEN].copy_from_slice(&tag);
    Ok(HEADER_LEN + body_len + TAG_LEN)
}

/// Reads the plaintext header of a packet whose first byte must be `format`.
pub(crate) fn peek_format(format: u8, packet: &[u8]) -> Result<Header, Error> {
    if packet.len() < HEADER_LEN + TAG_LEN {
        return Err(Error::Truncated);
    }
    if packet[0] != format {
        return Err(Error::BadVersion(packet[0]));
    }
    Ok(Header {
        version: packet[0],
        device_id: u32::from_le_bytes(packet[1..5].try_into().unwrap()),
        seq: u32::from_le_bytes(packet[5..9].try_into().unwrap()),
    })
}

/// Verifies and decrypts a packet sealed by [`seal_body`]; `N` bounds its size.
pub(crate) fn open_body<B: DeserializeOwned, const N: usize>(
    format: u8,
    domain: u32,
    key: &[u8; 32],
    packet: &[u8],
) -> Result<(Header, B), Error> {
    let header = peek_format(format, packet)?;
    if packet.len() > N {
        return Err(Error::Decode);
    }
    let (aad, rest) = packet.split_at(HEADER_LEN);
    let (ciphertext, tag) = rest.split_at(rest.len() - TAG_LEN);

    let mut buf = [0u8; N];
    let body_bytes = &mut buf[..ciphertext.len()];
    body_bytes.copy_from_slice(ciphertext);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt_in_place_detached(
            &nonce(header.device_id, header.seq, domain),
            aad,
            body_bytes,
            Tag::from_slice(tag),
        )
        .map_err(|_| Error::AuthFailed)?;
    let body = postcard::from_bytes(body_bytes).map_err(|_| Error::Decode)?;
    Ok((header, body))
}

/// Encrypts `report` with `key` into `out`, returning the packet length.
pub fn seal(key: &[u8; 32], report: &Report, out: &mut [u8]) -> Result<usize, Error> {
    let body = Body {
        ts: report.ts,
        lat_e7: report.lat_e7,
        lon_e7: report.lon_e7,
        alt_m: report.alt_m,
        speed_cms: report.speed_cms,
        hdop_x10: report.hdop_x10,
        batt_mv: report.batt_mv,
        flags: report.flags,
    };
    seal_body(VERSION, 0, key, report.device_id, report.seq, &body, out)
}

/// Reads the plaintext header so the caller can look up the device key.
pub fn peek_header(packet: &[u8]) -> Result<Header, Error> {
    peek_format(VERSION, packet)
}

/// Verifies and decrypts `packet`. Does not check replay; see [`ReplayGuard`].
pub fn open(key: &[u8; 32], packet: &[u8]) -> Result<Report, Error> {
    let (header, body): (Header, Body) = open_body::<_, MAX_PACKET>(VERSION, 0, key, packet)?;
    Ok(Report {
        device_id: header.device_id,
        seq: header.seq,
        ts: body.ts,
        lat_e7: body.lat_e7,
        lon_e7: body.lon_e7,
        alt_m: body.alt_m,
        speed_cms: body.speed_cms,
        hdop_x10: body.hdop_x10,
        batt_mv: body.batt_mv,
        flags: body.flags,
    })
}

/// Per-device replay protection: accepts only strictly increasing sequence numbers.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReplayGuard {
    last: Option<u32>,
}

impl ReplayGuard {
    /// Accepts `seq` if it is newer than anything seen, and remembers it.
    /// Call only after [`open`] succeeds, so forged packets can't advance it.
    pub fn accept(&mut self, seq: u32) -> Result<(), Error> {
        match self.last {
            Some(last) if seq <= last => Err(Error::Replay),
            _ => {
                self.last = Some(seq);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    /// A realistic report with extreme-ish values to exercise varints.
    fn sample() -> Report {
        Report {
            device_id: 0xDEAD_BEEF,
            seq: 42,
            ts: 1_793_000_000,
            lat_e7: 397_392_000,
            lon_e7: -1_049_903_000,
            alt_m: 1_609,
            speed_cms: 2_900,
            hdop_x10: 9,
            batt_mv: 4_100,
            flags: FLAG_STATIONARY,
        }
    }

    #[test]
    fn round_trip() {
        let mut buf = [0u8; MAX_PACKET];
        let n = seal(&KEY, &sample(), &mut buf).unwrap();
        assert!(n <= MAX_PACKET);
        assert_eq!(open(&KEY, &buf[..n]).unwrap(), sample());
    }

    #[test]
    fn worst_case_fits_max_packet() {
        let r = Report {
            device_id: u32::MAX,
            seq: u32::MAX,
            ts: u32::MAX,
            lat_e7: i32::MIN,
            lon_e7: i32::MIN,
            alt_m: i16::MIN,
            speed_cms: u16::MAX,
            hdop_x10: u8::MAX,
            batt_mv: u16::MAX,
            flags: u8::MAX,
        };
        let mut buf = [0u8; MAX_PACKET];
        let n = seal(&KEY, &r, &mut buf).unwrap();
        assert_eq!(open(&KEY, &buf[..n]).unwrap(), r);
    }

    #[test]
    fn header_is_readable_without_key() {
        let mut buf = [0u8; MAX_PACKET];
        let n = seal(&KEY, &sample(), &mut buf).unwrap();
        let h = peek_header(&buf[..n]).unwrap();
        assert_eq!((h.version, h.device_id, h.seq), (VERSION, 0xDEAD_BEEF, 42));
    }

    #[test]
    fn wrong_key_fails() {
        let mut buf = [0u8; MAX_PACKET];
        let n = seal(&KEY, &sample(), &mut buf).unwrap();
        assert_eq!(open(&[8u8; 32], &buf[..n]), Err(Error::AuthFailed));
    }

    #[test]
    fn any_flipped_bit_fails() {
        let mut buf = [0u8; MAX_PACKET];
        let n = seal(&KEY, &sample(), &mut buf).unwrap();
        // Skip byte 0: flipping the version is reported as BadVersion instead.
        for i in 1..n {
            let mut bad = buf;
            bad[i] ^= 0x01;
            assert_eq!(open(&KEY, &bad[..n]), Err(Error::AuthFailed), "byte {i}");
        }
    }

    #[test]
    fn bad_version_and_truncation() {
        let mut buf = [0u8; MAX_PACKET];
        let n = seal(&KEY, &sample(), &mut buf).unwrap();
        let mut bad = buf;
        bad[0] = 9;
        assert_eq!(open(&KEY, &bad[..n]), Err(Error::BadVersion(9)));
        assert_eq!(open(&KEY, &buf[..HEADER_LEN + TAG_LEN - 1]), Err(Error::Truncated));
    }

    #[test]
    fn small_output_buffer() {
        let mut tiny = [0u8; HEADER_LEN + TAG_LEN];
        assert_eq!(seal(&KEY, &sample(), &mut tiny), Err(Error::BufferTooSmall));
    }

    #[test]
    fn replay_guard() {
        let mut g = ReplayGuard::default();
        assert!(g.accept(5).is_ok());
        assert_eq!(g.accept(5), Err(Error::Replay));
        assert_eq!(g.accept(4), Err(Error::Replay));
        assert!(g.accept(6).is_ok());
    }
}
