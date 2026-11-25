//! Builds the telemetry heartbeat from the board's status.

use tracker_proto::config::Role;
use tracker_proto::telemetry::{BoardEvent, Gps as TGps, LocSource, Position, Telemetry, MAX_EVENTS};

use crate::location::{self, Source};
use crate::log::EventLog;
use crate::screens::{Gps, Status};

/// Seconds between heartbeats while WiFi is up.
pub const EVERY_S: u32 = 60;

/// A snapshot of `s` for the server, plus the newest log lines after event
/// id `sent` (what the server already has). `board` is the Heltec revision.
/// Returns the id of the newest line included, to pass as `sent` next time
/// once this heartbeat got through.
pub fn from_status<const N: usize>(
    s: &Status,
    log: &EventLog<N>,
    sent: Option<u32>,
    board: u8,
    boot: u16,
) -> (Telemetry, Option<u32>) {
    let fresh = log.after(sent).count();
    let mut events = heapless::Vec::new();
    let mut newest = sent;
    for e in log.after(sent).skip(fresh.saturating_sub(MAX_EVENTS)) {
        let _ = events.push(BoardEvent { age_s: s.uptime_s.saturating_sub(e.at_s), text: e.text.clone() });
        newest = Some(e.id);
    }
    let position = location::best(s.gps_loc.as_ref(), s.ip_loc.as_ref(), s.uptime_s).map(|b| Position {
        source: match b.loc.source {
            Source::Gps => LocSource::Gps,
            Source::Ip => LocSource::Ip,
        },
        lat_e7: b.loc.lat_e7,
        lon_e7: b.loc.lon_e7,
        age_s: s.uptime_s.saturating_sub(b.loc.at_s),
    });
    let t = Telemetry {
        role: s.role.unwrap_or(Role::Tracker),
        board,
        ts: s.unix().unwrap_or(0),
        uptime_s: s.uptime_s,
        boot,
        batt_mv: s.batt_mv,
        gps: match s.gps {
            Gps::Absent => TGps::Absent,
            Gps::Searching => TGps::Searching,
            Gps::Fix => TGps::Fix,
        },
        sats: s.sats,
        wifi_rssi: s.wifi_rssi,
        lora_tx: s.lora_tx,
        lora_rx: s.lora_rx,
        lora_bad: s.lora_bad,
        lora_rssi: s.lora_rssi,
        sent_ok: s.sent_ok,
        sent_fail: s.sent_fail,
        queued: s.queued,
        position,
        events,
    };
    (t, newest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Location;

    #[test]
    fn carries_status_and_best_position() {
        let mut s = Status {
            role: Some(Role::Tracker),
            uptime_s: 500,
            clock_base: Some(1_000),
            batt_mv: Some(3900),
            wifi_rssi: Some(-50),
            lora_tx: 3,
            queued: 2,
            ..Default::default()
        };
        let mut log: EventLog<16> = EventLog::new();
        let (t, _) = from_status(&s, &log, None, 2, 7);
        assert_eq!((t.board, t.boot, t.ts, t.uptime_s), (2, 7, 1_500, 500));
        assert_eq!((t.gps, t.batt_mv, t.wifi_rssi, t.lora_tx, t.queued), (TGps::Absent, Some(3900), Some(-50), 3, 2));
        assert_eq!(t.position, None, "phones home even with no position");

        s.ip_loc = Some(Location {
            source: Source::Ip,
            lat_e7: 10,
            lon_e7: 20,
            alt_m: 0,
            speed_cms: 0,
            hdop_x10: 0,
            sats: 0,
            at_s: 440,
            place: Default::default(),
        });
        let p = from_status(&s, &log, None, 2, 7).0.position.unwrap();
        assert_eq!((p.source, p.lat_e7, p.lon_e7, p.age_s), (LocSource::Ip, 10, 20, 60));

        for i in 0..10 {
            log.push(400 + i, format_args!("event {i}"));
        }
        let (t, newest) = from_status(&s, &log, None, 2, 7);
        let texts: Vec<_> = t.events.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["event 4", "event 5", "event 6", "event 7", "event 8", "event 9"]);
        assert_eq!((t.events[0].age_s, newest), (96, Some(9)));
        assert!(from_status(&s, &log, newest, 2, 7).0.events.is_empty());
    }
}
