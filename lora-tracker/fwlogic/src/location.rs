//! Picks the best position the board has and turns it into report fields.
//!
//! Order: a current GPS fix, then a recent IP estimate, then whatever was
//! last known (flagged `NO_FIX`). IP positions always carry `FLAG_APPROX`.

use heapless::String;
use tracker_proto::config::LoraConfig;
use tracker_proto::packet::{Report, FLAG_APPROX, FLAG_NO_FIX, FLAG_STATIONARY};

/// A GPS fix older than this is no longer "current".
pub const GPS_CURRENT_S: u32 = 10;
/// An IP estimate is trusted as current for this long.
pub const IP_CURRENT_S: u32 = 6 * 3600;
/// Retry an IP lookup this often after a success...
pub const IP_RETRY_OK_S: u32 = 30 * 60;
/// ...and this often after a failure.
pub const IP_RETRY_FAIL_S: u32 = 5 * 60;
/// Below this GPS ground speed the tracker counts as stationary.
pub const STATIONARY_CMS: u16 = 100;
/// Report intervals when there's no `[lora]` section to take them from.
const DEFAULT_INTERVALS: (u16, u16) = (60, 600);

/// Where a position came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Gps,
    /// IP geolocation: city-level, and of the network, not the board.
    Ip,
}

/// One position, stamped with the uptime it was taken at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub source: Source,
    pub lat_e7: i32,
    pub lon_e7: i32,
    pub alt_m: i16,
    pub speed_cms: u16,
    pub hdop_x10: u8,
    pub sats: u8,
    /// Board uptime (s) when this was measured.
    pub at_s: u32,
    /// City name for IP estimates; empty for GPS.
    pub place: String<20>,
}

/// The position to report, and whether it is still current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Best<'a> {
    pub loc: &'a Location,
    pub current: bool,
}

/// Whether `gps` is a current fix at `now_s`.
pub fn gps_current(gps: Option<&Location>, now_s: u32) -> bool {
    gps.is_some_and(|g| now_s.saturating_sub(g.at_s) <= GPS_CURRENT_S)
}

/// Chooses between the last GPS fix and the last IP estimate at `now_s`.
pub fn best<'a>(gps: Option<&'a Location>, ip: Option<&'a Location>, now_s: u32) -> Option<Best<'a>> {
    let age = |l: &Location| now_s.saturating_sub(l.at_s);
    if let Some(g) = gps.filter(|_| gps_current(gps, now_s)) {
        return Some(Best { loc: g, current: true });
    }
    if let Some(i) = ip.filter(|i| age(i) <= IP_CURRENT_S) {
        return Some(Best { loc: i, current: true });
    }
    let newest = match (gps, ip) {
        (Some(g), Some(i)) => Some(if g.at_s >= i.at_s { g } else { i }),
        (g, i) => g.or(i),
    };
    newest.map(|loc| Best { loc, current: false })
}

/// Copies the position into `r` and sets its flags.
pub fn fill(r: &mut Report, best: Option<Best<'_>>) {
    r.flags = 0;
    let Some(Best { loc, current }) = best else {
        r.flags = FLAG_NO_FIX;
        return;
    };
    r.lat_e7 = loc.lat_e7;
    r.lon_e7 = loc.lon_e7;
    r.alt_m = loc.alt_m;
    r.speed_cms = if current { loc.speed_cms } else { 0 };
    r.hdop_x10 = loc.hdop_x10;
    if !current {
        r.flags |= FLAG_NO_FIX;
    }
    if loc.source == Source::Ip {
        r.flags |= FLAG_APPROX;
    } else if current && loc.speed_cms < STATIONARY_CMS {
        r.flags |= FLAG_STATIONARY;
    }
}

/// Whether to (re)try an IP lookup now. Never while GPS has a current fix.
/// `last` is the uptime of the previous attempt and whether it succeeded.
pub fn ip_lookup_due(gps_current: bool, last: Option<(u32, bool)>, now_s: u32) -> bool {
    match (gps_current, last) {
        (true, _) => false,
        (false, None) => true,
        (false, Some((at, ok))) => {
            now_s.saturating_sub(at) >= if ok { IP_RETRY_OK_S } else { IP_RETRY_FAIL_S }
        }
    }
}

/// Seconds until the next report: the fast interval only while GPS says
/// we're moving. A board without GPS can't tell, so it reports slowly.
pub fn interval_s(flags: u8, lora: Option<&LoraConfig>) -> u16 {
    let (moving, still) = lora.map_or(DEFAULT_INTERVALS, |l| (l.interval_s, l.stationary_interval_s));
    let gps_moving = flags & (FLAG_NO_FIX | FLAG_APPROX | FLAG_STATIONARY) == 0;
    if gps_moving { moving } else { still }.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(source: Source, at_s: u32, speed_cms: u16) -> Location {
        Location {
            source,
            lat_e7: 1,
            lon_e7: 2,
            alt_m: 3,
            speed_cms,
            hdop_x10: 9,
            sats: 7,
            at_s,
            place: String::new(),
        }
    }

    #[test]
    fn current_gps_beats_ip() {
        let (g, i) = (loc(Source::Gps, 100, 0), loc(Source::Ip, 105, 0));
        assert_eq!(best(Some(&g), Some(&i), 105).unwrap().loc.source, Source::Gps);
        assert!(gps_current(Some(&g), 100 + GPS_CURRENT_S) && !gps_current(Some(&g), 101 + GPS_CURRENT_S));
    }

    #[test]
    fn ip_fills_in_when_gps_is_stale_or_missing() {
        let (g, i) = (loc(Source::Gps, 100, 0), loc(Source::Ip, 50, 0));
        let b = best(Some(&g), Some(&i), 200).unwrap();
        assert_eq!((b.loc.source, b.current), (Source::Ip, true));
        assert_eq!(best(None, Some(&i), 60).unwrap().loc.source, Source::Ip);
    }

    #[test]
    fn falls_back_to_newest_stale_position() {
        let (g, i) = (loc(Source::Gps, 100, 0), loc(Source::Ip, 50, 0));
        let b = best(Some(&g), Some(&i), 50 + IP_CURRENT_S + 1).unwrap();
        assert_eq!((b.loc.source, b.current), (Source::Gps, false));
        assert_eq!(best(None, None, 5), None);
    }

    #[test]
    fn flags_follow_the_source() {
        let mut r = Report::default();
        let moving = loc(Source::Gps, 0, 500);
        fill(&mut r, Some(Best { loc: &moving, current: true }));
        assert_eq!((r.flags, r.speed_cms), (0, 500));

        let parked = loc(Source::Gps, 0, 20);
        fill(&mut r, Some(Best { loc: &parked, current: true }));
        assert_eq!(r.flags, FLAG_STATIONARY);

        let ip = loc(Source::Ip, 0, 0);
        fill(&mut r, Some(Best { loc: &ip, current: true }));
        assert_eq!(r.flags, FLAG_APPROX);
        fill(&mut r, Some(Best { loc: &ip, current: false }));
        assert_eq!(r.flags, FLAG_APPROX | FLAG_NO_FIX);

        fill(&mut r, Some(Best { loc: &moving, current: false }));
        assert_eq!((r.flags, r.speed_cms), (FLAG_NO_FIX, 0));
        fill(&mut r, None);
        assert_eq!(r.flags, FLAG_NO_FIX);
    }

    #[test]
    fn ip_lookups_back_off_and_stop_with_gps() {
        assert!(ip_lookup_due(false, None, 0));
        assert!(!ip_lookup_due(true, None, 0));
        assert!(!ip_lookup_due(false, Some((0, false)), IP_RETRY_FAIL_S - 1));
        assert!(ip_lookup_due(false, Some((0, false)), IP_RETRY_FAIL_S));
        assert!(!ip_lookup_due(false, Some((0, true)), IP_RETRY_FAIL_S));
        assert!(ip_lookup_due(false, Some((0, true)), IP_RETRY_OK_S));
    }

    #[test]
    fn only_moving_gps_uses_the_fast_interval() {
        let l = LoraConfig { frequency_hz: 915_000_000, tx_power_dbm: 14, interval_s: 30, stationary_interval_s: 600 };
        assert_eq!(interval_s(0, Some(&l)), 30);
        assert_eq!(interval_s(FLAG_STATIONARY, Some(&l)), 600);
        assert_eq!(interval_s(FLAG_APPROX, Some(&l)), 600);
        assert_eq!(interval_s(FLAG_NO_FIX, None), 600);
        assert_eq!(interval_s(0, None), 60);
    }
}
