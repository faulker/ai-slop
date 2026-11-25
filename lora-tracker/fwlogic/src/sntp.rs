//! Minimal SNTP (RFC 4330) client packets, for setting the clock when there's
//! no GPS time.

/// NTP server to ask, on UDP port 123.
pub const HOST: &str = "pool.ntp.org";
/// Seconds between the NTP epoch (1900) and the unix epoch.
const NTP_TO_UNIX: u32 = 2_208_988_800;

/// A client request: version 4, mode 3 (client), everything else zero.
pub fn request() -> [u8; 48] {
    let mut p = [0u8; 48];
    p[0] = 0x23;
    p
}

/// Unix seconds from a server reply, or `None` for anything that isn't a
/// usable server answer (wrong mode, kiss-of-death stratum 0, unsynced).
pub fn parse(resp: &[u8]) -> Option<u32> {
    if resp.len() < 48 || resp[0] & 0x07 != 4 || resp[1] == 0 || resp[0] >> 6 == 3 {
        return None;
    }
    let secs = u32::from_be_bytes(resp[40..44].try_into().ok()?);
    secs.checked_sub(NTP_TO_UNIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(secs: u32) -> [u8; 48] {
        let mut p = [0u8; 48];
        p[0] = 0x24; // LI 0, v4, mode 4
        p[1] = 2;
        p[40..44].copy_from_slice(&secs.to_be_bytes());
        p
    }

    #[test]
    fn reads_transmit_time() {
        assert_eq!(request()[0] & 0x07, 3);
        assert_eq!(parse(&reply(NTP_TO_UNIX + 1_700_000_000)), Some(1_700_000_000));
    }

    #[test]
    fn rejects_bad_replies() {
        let mut kod = reply(NTP_TO_UNIX + 5);
        kod[1] = 0;
        assert_eq!(parse(&kod), None);
        let mut unsynced = reply(NTP_TO_UNIX + 5);
        unsynced[0] |= 0xC0;
        assert_eq!(parse(&unsynced), None);
        assert_eq!(parse(&request()), None);
        assert_eq!(parse(&[0u8; 10]), None);
    }
}
