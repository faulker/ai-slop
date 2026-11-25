//! Text for the 128x64 OLED: 6 lines of 21 characters (6x10 font).
//!
//! Pages, cycled by the PRG button (and automatically when idle):
//! [`Page::Stats`], [`Page::Location`], [`Page::LastConn`], [`Page::Events`].

use core::fmt::Write;

use heapless::{String, Vec};
use tracker_proto::config::Role;

use crate::location::{self, Location, Source};
use crate::log::{EventLog, Truncate};
use crate::time;

/// Characters per line.
pub const COLS: usize = 21;
/// Lines per page.
pub const ROWS: usize = 6;

pub type Line = String<COLS>;
pub type Lines = Vec<Line, ROWS>;

/// Which page is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Stats,
    Location,
    LastConn,
    Events,
}

impl Page {
    /// The page after this one, wrapping around.
    pub fn next(self) -> Page {
        match self {
            Page::Stats => Page::Location,
            Page::Location => Page::LastConn,
            Page::LastConn => Page::Events,
            Page::Events => Page::Stats,
        }
    }
}

/// State of one uplink, for the stats page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Link {
    /// Not configured (see `lora-tracker show`).
    #[default]
    Off,
    /// Configured, but the hardware didn't answer or there's no network.
    Down,
    /// Joining a network or initialising.
    Starting,
    Up,
}

/// GPS module state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gps {
    /// No NMEA seen: nothing plugged in (or wrong pins/baud).
    #[default]
    Absent,
    /// Talking, but no fix yet.
    Searching,
    Fix,
}

/// The most recent exchange on one link, in three ready-made lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastConn {
    pub at_s: u32,
    /// After the link name and age, e.g. the SSID or `tx #12`.
    pub what: String<COLS>,
    /// e.g. RSSI and IP address.
    pub detail: String<COLS>,
    /// e.g. `POST 204 x3` or the error.
    pub result: String<COLS>,
}

/// Everything the pages show, updated by the firmware tasks.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub role: Option<Role>,
    pub device_id: u32,
    /// Why the config couldn't be loaded; `None` when it was.
    pub config_error: Option<&'static str>,
    pub uptime_s: u32,
    /// `unix - uptime`, once GPS or SNTP has told us the time.
    pub clock_base: Option<u32>,
    pub batt_mv: Option<u16>,
    pub gps: Gps,
    pub sats: u8,
    pub lora: Link,
    pub lora_tx: u32,
    pub lora_rx: u32,
    /// Received packets that failed to decode or verify.
    pub lora_bad: u32,
    pub lora_rssi: Option<i16>,
    pub lora_snr: Option<i16>,
    pub wifi: Link,
    pub wifi_rssi: Option<i8>,
    pub sent_ok: u32,
    pub sent_fail: u32,
    pub queued: u16,
    pub last_wifi: Option<LastConn>,
    pub last_lora: Option<LastConn>,
    pub gps_loc: Option<Location>,
    pub ip_loc: Option<Location>,
}

impl Status {
    /// Current unix time, if known.
    pub fn unix(&self) -> Option<u32> {
        self.clock_base.map(|b| b + self.uptime_s)
    }
}

/// Renders one page.
pub fn render<const N: usize>(page: Page, s: &Status, log: &EventLog<N>) -> Lines {
    let mut out = Lines::new();
    if let Some(err) = s.config_error {
        line(&mut out, format_args!("NO CONFIG ({err})"));
        line(&mut out, format_args!("Run on your computer:"));
        line(&mut out, format_args!(" lora-tracker"));
        line(&mut out, format_args!("  flash-config <toml>"));
        line(&mut out, format_args!("up {}", time::uptime(s.uptime_s)));
        return out;
    }
    match page {
        Page::Stats => stats(&mut out, s),
        Page::Location => location_page(&mut out, s),
        Page::LastConn => last_conn(&mut out, s),
        Page::Events => events(&mut out, s, log),
    }
    out
}

/// Appends one truncated line, if there's room on the page.
fn line(out: &mut Lines, args: core::fmt::Arguments<'_>) {
    let mut l = Line::new();
    let _ = Truncate(&mut l).write_fmt(args);
    let _ = out.push(l);
}

/// Identity, uptime, battery, per-link state and upload counters.
fn stats(out: &mut Lines, s: &Status) {
    let role = match s.role {
        Some(Role::Tracker) => "TRK",
        Some(Role::Gateway) => "GW",
        None => "?",
    };
    match s.unix() {
        Some(t) => {
            let (h, m) = time::hh_mm(t);
            line(out, format_args!("{role} {} {h:02}:{m:02}Z", s.device_id));
        }
        None => line(out, format_args!("{role} {}", s.device_id)),
    }
    let mut batt = String::<8>::new();
    let _ = match s.batt_mv {
        Some(mv) => write!(batt, "{}.{:02}V", mv / 1000, mv % 1000 / 10),
        None => write!(batt, "--"),
    };
    line(out, format_args!("up {} batt {batt}", time::uptime(s.uptime_s)));
    match s.gps {
        Gps::Absent => line(out, format_args!("GPS none")),
        Gps::Searching => line(out, format_args!("GPS search {}sat", s.sats)),
        Gps::Fix => line(out, format_args!("GPS fix {}sat", s.sats)),
    }
    let mut sig = String::<12>::new();
    if let Some(r) = s.lora_rssi {
        let _ = write!(sig, " {r}dBm");
    }
    match (s.lora, s.role) {
        (Link::Up, Some(Role::Gateway)) => line(out, format_args!("LoRa rx{} bad{}{sig}", s.lora_rx, s.lora_bad)),
        (Link::Up, _) => line(out, format_args!("LoRa tx {}", s.lora_tx)),
        (l, _) => line(out, format_args!("LoRa {}", link_word(l))),
    }
    match (s.wifi, s.wifi_rssi) {
        (Link::Up, Some(r)) => line(out, format_args!("WiFi up {r}dBm")),
        (l, _) => line(out, format_args!("WiFi {}", link_word(l))),
    }
    line(out, format_args!("sent {} fail {} q{}", s.sent_ok, s.sent_fail, s.queued));
}

/// The word for a link that isn't showing counters.
fn link_word(l: Link) -> &'static str {
    match l {
        Link::Off => "off",
        Link::Down => "down",
        Link::Starting => "starting",
        Link::Up => "up",
    }
}

/// The position the board would report right now, and where it came from.
fn location_page(out: &mut Lines, s: &Status) {
    let Some(b) = location::best(s.gps_loc.as_ref(), s.ip_loc.as_ref(), s.uptime_s) else {
        line(out, format_args!("LOCATION unknown"));
        match s.gps {
            Gps::Absent => line(out, format_args!("no GPS; IP lookup")),
            _ => line(out, format_args!("GPS searching")),
        }
        line(out, format_args!("needs WiFi + internet"));
        return;
    };
    let l = b.loc;
    let age = time::age(s.uptime_s.saturating_sub(l.at_s));
    match (l.source, b.current) {
        (Source::Gps, true) => line(out, format_args!("LOCATION via GPS")),
        (Source::Ip, true) => line(out, format_args!("LOCATION via IP ~")),
        (Source::Gps, false) => line(out, format_args!("LAST GPS {age} ago")),
        (Source::Ip, false) => line(out, format_args!("LAST IP {age} ago")),
    }
    line(out, format_args!(" {}", Coord(l.lat_e7, 'N', 'S')));
    line(out, format_args!(" {}", Coord(l.lon_e7, 'E', 'W')));
    match l.source {
        Source::Gps => {
            line(out, format_args!("alt {}m {} km/h", l.alt_m, l.speed_cms as u32 * 36 / 1000));
            line(out, format_args!("{} sat hdop {}.{}", l.sats, l.hdop_x10 / 10, l.hdop_x10 % 10));
        }
        Source::Ip => {
            line(out, format_args!("{}", if l.place.is_empty() { "(no city)" } else { &l.place }));
            line(out, format_args!("city-level, of ISP"));
        }
    }
    line(out, format_args!("measured {age} ago"));
}

/// Degrees with 5 decimals (about 1 m) and a hemisphere letter.
struct Coord(i32, char, char);

impl core::fmt::Display for Coord {
    /// Writes e.g. `30.26720 N`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let a = self.0.unsigned_abs();
        let hemi = if self.0 < 0 { self.2 } else { self.1 };
        write!(f, "{}.{:05} {hemi}", a / 10_000_000, a % 10_000_000 / 100)
    }
}

/// The last WiFi session and the last LoRa packet, three lines each.
fn last_conn(out: &mut Lines, s: &Status) {
    for (name, link, last) in [("WiFi", s.wifi, &s.last_wifi), ("LoRa", s.lora, &s.last_lora)] {
        match last {
            Some(c) => {
                let age = time::age(s.uptime_s.saturating_sub(c.at_s));
                line(out, format_args!("{name} {age} {}", c.what));
                line(out, format_args!(" {}", c.detail));
                line(out, format_args!(" {}", c.result));
            }
            None if link == Link::Off => line(out, format_args!("{name}: off")),
            None => line(out, format_args!("{name}: none yet")),
        }
    }
}

/// The newest events, each prefixed with its age.
fn events<const N: usize>(out: &mut Lines, s: &Status, log: &EventLog<N>) {
    let mut any = false;
    for e in log.newest().take(ROWS) {
        any = true;
        let age = time::age(s.uptime_s.saturating_sub(e.at_s));
        line(out, format_args!("{age:>3} {}", e.text));
    }
    if !any {
        line(out, format_args!("no events yet"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &Lines) -> std::vec::Vec<&str> {
        lines.iter().map(|l| l.as_str()).collect()
    }

    fn tracker() -> Status {
        Status {
            role: Some(Role::Tracker),
            device_id: 42,
            uptime_s: 3720,
            lora: Link::Up,
            lora_tx: 12,
            wifi: Link::Up,
            wifi_rssi: Some(-61),
            sent_ok: 10,
            sent_fail: 1,
            queued: 2,
            ..Default::default()
        }
    }

    #[test]
    fn stats_page() {
        let mut s = tracker();
        s.clock_base = Some(764_426_119 - 3720);
        s.batt_mv = Some(3921);
        let log: EventLog<4> = EventLog::new();
        assert_eq!(
            text(&render(Page::Stats, &s, &log)),
            ["TRK 42 12:35Z", "up 1h02m batt 3.92V", "GPS none", "LoRa tx 12", "WiFi up -61dBm", "sent 10 fail 1 q2"]
        );
    }

    #[test]
    fn gateway_shows_receive_counters() {
        let s = Status {
            role: Some(Role::Gateway),
            lora: Link::Up,
            lora_rx: 5,
            lora_bad: 1,
            lora_rssi: Some(-87),
            ..tracker()
        };
        let log: EventLog<4> = EventLog::new();
        assert_eq!(render(Page::Stats, &s, &log)[3].as_str(), "LoRa rx5 bad1 -87dBm");
    }

    #[test]
    fn location_page_explains_ip_fallback() {
        let mut s = tracker();
        let log: EventLog<4> = EventLog::new();
        assert_eq!(render(Page::Location, &s, &log)[0].as_str(), "LOCATION unknown");

        let mut place = String::new();
        place.push_str("Austin").unwrap();
        s.ip_loc = Some(Location {
            source: Source::Ip,
            lat_e7: 302_672_000,
            lon_e7: -977_431_000,
            alt_m: 0,
            speed_cms: 0,
            hdop_x10: 0,
            sats: 0,
            at_s: 3600,
            place,
        });
        assert_eq!(
            text(&render(Page::Location, &s, &log)),
            ["LOCATION via IP ~", " 30.26720 N", " 97.74310 W", "Austin", "city-level, of ISP", "measured 2m ago"]
        );
    }

    #[test]
    fn last_connection_page() {
        let mut s = tracker();
        s.lora = Link::Off;
        s.last_wifi = Some(LastConn {
            at_s: 3600,
            what: "SimmerDown".try_into().unwrap(),
            detail: "-61dBm 192.168.1.50".try_into().unwrap(),
            result: "POST 204 x3".try_into().unwrap(),
        });
        let log: EventLog<4> = EventLog::new();
        assert_eq!(
            text(&render(Page::LastConn, &s, &log)),
            ["WiFi 2m SimmerDown", " -61dBm 192.168.1.50", " POST 204 x3", "LoRa: off"]
        );
    }

    #[test]
    fn events_page_is_newest_first_and_fits() {
        let s = tracker();
        let mut log: EventLog<8> = EventLog::new();
        log.push(3000, format_args!("wifi up SimmerDown"));
        log.push(3715, format_args!("a very long event that overflows"));
        let lines = render(Page::Events, &s, &log);
        assert_eq!(lines[0].as_str(), " 5s a very long event");
        assert_eq!(lines[1].as_str(), "12m wifi up SimmerDow");
        assert!(lines.iter().all(|l| l.len() <= COLS));
    }

    #[test]
    fn missing_config_takes_over_every_page() {
        let s = Status { config_error: Some("blank"), ..tracker() };
        let log: EventLog<4> = EventLog::new();
        for p in [Page::Stats, Page::Location, Page::LastConn, Page::Events] {
            assert!(render(p, &s, &log)[0].starts_with("NO CONFIG"));
        }
        assert_eq!(Page::Events.next(), Page::Stats);
    }
}
