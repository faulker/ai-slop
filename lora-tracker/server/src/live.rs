//! Live view of the boards, kept in memory: an event log (printed to the
//! terminal, kept for the `/log` page, streamed over `GET /api/events`) and
//! each device's last contact and telemetry. Nothing here survives a restart;
//! reports are what's stored in SQLite.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde::Serialize;
use tokio::sync::broadcast;
use tracker_proto::config::Role;
use tracker_proto::packet::ReplayGuard;
use tracker_proto::telemetry::{Gps, LocSource, Telemetry};

/// Events kept for clients that connect later.
const KEEP: usize = 300;
/// A device heard from within this many seconds counts as online. Boards send
/// telemetry every minute while on WiFi.
pub const ONLINE_S: i64 = 150;

/// How much an event matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    /// Something was rejected or failed.
    Warn,
}

/// One line of the log.
#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    /// Server clock, unix seconds.
    pub at: i64,
    pub level: Level,
    /// Where the request came from.
    pub addr: String,
    pub device_id: Option<u32>,
    pub text: String,
}

/// Telemetry in API units.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TelemetryView {
    pub role: &'static str,
    pub board: String,
    pub uptime_s: u32,
    pub boot: u16,
    /// The board's own clock (unix s), 0 if unset.
    pub device_ts: u32,
    pub batt_v: Option<f64>,
    pub gps: &'static str,
    pub sats: u8,
    pub wifi_rssi: Option<i8>,
    pub lora_tx: u32,
    pub lora_rx: u32,
    pub lora_bad: u32,
    pub lora_rssi: Option<i16>,
    pub sent_ok: u32,
    pub sent_fail: u32,
    pub queued: u16,
    pub position: Option<PositionView>,
}

/// The board's best position.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PositionView {
    /// `"gps"` or `"ip"` (city-level).
    pub source: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub age_s: u32,
}

impl From<&Telemetry> for TelemetryView {
    /// Converts fixed-point and enum fields to JSON-friendly ones.
    fn from(t: &Telemetry) -> Self {
        TelemetryView {
            role: match t.role {
                Role::Tracker => "tracker",
                Role::Gateway => "gateway",
            },
            board: format!("Heltec V{}", t.board),
            uptime_s: t.uptime_s,
            boot: t.boot,
            device_ts: t.ts,
            batt_v: t.batt_mv.map(|mv| mv as f64 / 1000.0),
            gps: match t.gps {
                Gps::Absent => "none",
                Gps::Searching => "searching",
                Gps::Fix => "fix",
            },
            sats: t.sats,
            wifi_rssi: t.wifi_rssi,
            lora_tx: t.lora_tx,
            lora_rx: t.lora_rx,
            lora_bad: t.lora_bad,
            lora_rssi: t.lora_rssi,
            sent_ok: t.sent_ok,
            sent_fail: t.sent_fail,
            queued: t.queued,
            position: t.position.map(|p| PositionView {
                source: match p.source {
                    LocSource::Gps => "gps",
                    LocSource::Ip => "ip",
                },
                lat: p.lat_e7 as f64 / 1e7,
                lon: p.lon_e7 as f64 / 1e7,
                age_s: p.age_s,
            }),
        }
    }
}

impl TelemetryView {
    /// One-line summary for the log.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("{} {}", self.board, self.role), format!("up {}", uptime(self.uptime_s))];
        parts.push(match self.batt_v {
            Some(v) => format!("batt {v:.2} V"),
            None => "batt --".into(),
        });
        parts.push(match self.gps {
            "fix" => format!("gps fix {} sats", self.sats),
            g => format!("gps {g}"),
        });
        if let Some(r) = self.wifi_rssi {
            parts.push(format!("wifi {r} dBm"));
        }
        if self.role == "gateway" {
            parts.push(format!("lora rx {} bad {}", self.lora_rx, self.lora_bad));
        } else {
            parts.push(format!("lora tx {}", self.lora_tx));
        }
        parts.push(format!("sent {} failed {} queued {}", self.sent_ok, self.sent_fail, self.queued));
        parts.push(match &self.position {
            Some(p) => format!("at {:.5},{:.5} via {} ({} ago)", p.lat, p.lon, p.source, uptime(p.age_s)),
            None => "no position".into(),
        });
        parts.join(", ")
    }
}

/// `1h02m`-style duration.
fn uptime(s: u32) -> String {
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        3600..=86_399 => format!("{}h{:02}m", s / 3600, s / 60 % 60),
        _ => format!("{}d{:02}h", s / 86_400, s / 3600 % 24),
    }
}

/// The last contact with one device.
#[derive(Debug, Clone, Serialize)]
pub struct Seen {
    /// Server clock, unix seconds.
    pub last_seen: i64,
    pub addr: String,
    pub telemetry: Option<TelemetryView>,
}

/// Shared live state.
pub struct Live {
    events: Mutex<VecDeque<LogEvent>>,
    seen: Mutex<HashMap<u32, Seen>>,
    /// Per-device telemetry seq, so a captured heartbeat can't be replayed.
    /// Seq is `boot << 16 | n` with a persisted boot counter, so it only grows.
    guards: Mutex<HashMap<u32, ReplayGuard>>,
    tx: broadcast::Sender<LogEvent>,
    /// Off in tests, so `cargo test` output stays clean.
    print: bool,
}

impl Live {
    /// Empty state. `print` echoes every event to stdout.
    pub fn new(print: bool) -> Self {
        Live {
            events: Mutex::new(VecDeque::new()),
            seen: Mutex::new(HashMap::new()),
            guards: Mutex::new(HashMap::new()),
            tx: broadcast::channel(64).0,
            print,
        }
    }

    /// Accepts a verified telemetry `seq` only if it's newer than the last one
    /// from `device_id`. Call after the packet's tag checked out.
    pub fn accept_telemetry(&self, device_id: u32, seq: u32) -> bool {
        self.guards.lock().unwrap().entry(device_id).or_default().accept(seq).is_ok()
    }

    /// Records an event: prints it, keeps it, and sends it to live listeners.
    pub fn log(&self, at: i64, level: Level, addr: &str, device_id: Option<u32>, name: Option<&str>, text: String) {
        if self.print {
            let when = chrono::DateTime::from_timestamp(at, 0)
                .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();
            let who = match (name, device_id) {
                (Some(n), Some(id)) => format!("{n} ({id})"),
                (None, Some(id)) => format!("device {id}"),
                _ => "-".into(),
            };
            let mark = if level == Level::Warn { "WARN " } else { "" };
            println!("{when}  {addr:<21} {who}  {mark}{text}");
        }
        let ev = LogEvent { at, level, addr: addr.into(), device_id, text };
        let mut events = self.events.lock().unwrap();
        if events.len() == KEEP {
            events.pop_front();
        }
        events.push_back(ev.clone());
        // No listeners is fine.
        let _ = self.tx.send(ev);
    }

    /// Notes that `device_id` just reached us, with telemetry if it sent some.
    pub fn saw(&self, device_id: u32, at: i64, addr: &str, telemetry: Option<TelemetryView>) {
        let mut seen = self.seen.lock().unwrap();
        let entry = seen.entry(device_id).or_insert_with(|| Seen { last_seen: at, addr: addr.into(), telemetry: None });
        entry.last_seen = at;
        entry.addr = addr.into();
        if telemetry.is_some() {
            entry.telemetry = telemetry;
        }
    }

    /// The last contact with `device_id`, if any since the server started.
    pub fn seen(&self, device_id: u32) -> Option<Seen> {
        self.seen.lock().unwrap().get(&device_id).cloned()
    }

    /// Kept events, oldest first.
    pub fn recent(&self) -> Vec<LogEvent> {
        self.events.lock().unwrap().iter().cloned().collect()
    }

    /// A receiver for events from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<LogEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracker_proto::telemetry::{LocSource, Position};

    #[test]
    fn keeps_a_bounded_log_and_streams_it() {
        let live = Live::new(false);
        let mut rx = live.subscribe();
        live.log(0, Level::Info, "a", None, None, "e0".into());
        assert_eq!(rx.try_recv().unwrap().text, "e0");
        for i in 1..KEEP + 5 {
            live.log(i as i64, Level::Info, "a", None, None, format!("e{i}"));
        }
        let recent = live.recent();
        assert_eq!(recent.len(), KEEP);
        assert_eq!(recent[0].text, "e5");
    }

    #[test]
    fn saw_keeps_the_last_telemetry() {
        let live = Live::new(false);
        let t = Telemetry {
            role: Role::Tracker,
            board: 2,
            ts: 0,
            uptime_s: 3720,
            boot: 8,
            batt_mv: None,
            gps: Gps::Absent,
            sats: 0,
            wifi_rssi: Some(-52),
            lora_tx: 3,
            lora_rx: 0,
            lora_bad: 0,
            lora_rssi: None,
            sent_ok: 2,
            sent_fail: 0,
            queued: 1,
            position: Some(Position { source: LocSource::Ip, lat_e7: 400_000_000, lon_e7: -1_050_000_000, age_s: 90 }),
            events: Default::default(),
        };
        live.saw(1, 10, "x:1", Some(TelemetryView::from(&t)));
        live.saw(1, 20, "x:2", None);
        let s = live.seen(1).unwrap();
        assert_eq!((s.last_seen, s.addr.as_str()), (20, "x:2"));
        let v = s.telemetry.unwrap();
        assert_eq!(
            v.summary(),
            "Heltec V2 tracker, up 1h02m, batt --, gps none, wifi -52 dBm, lora tx 3, \
             sent 2 failed 0 queued 1, at 40.00000,-105.00000 via ip (1m30s ago)"
        );
        assert!(live.seen(2).is_none());
    }
}
