//! Optional external GPS on UART1 (NMEA at 9600 baud). With nothing wired up
//! the board simply reports `GPS none` and falls back to IP location.

use embassy_time::{Duration, with_timeout};
use esp_hal::Async;
use esp_hal::uart::UartRx;
use fwlogic::location::{Location, Source};
use fwlogic::nmea::Nmea;
use fwlogic::screens::Gps;

use crate::{event, state};

/// No valid sentence for this long means the module is gone.
const SILENT_S: u32 = 5;

/// Reads NMEA forever and keeps the shared GPS state and clock current.
#[embassy_executor::task]
pub async fn task(mut rx: UartRx<'static, Async>) {
    let mut nmea = Nmea::default();
    let mut buf = [0u8; 64];
    let mut last_good: Option<u32> = None;
    let mut shown = Gps::Absent;
    loop {
        if let Ok(Ok(n)) = with_timeout(Duration::from_secs(1), rx.read_async(&mut buf)).await {
            for b in &buf[..n] {
                if nmea.feed(*b) {
                    last_good = Some(state::uptime());
                }
            }
        }
        let now = state::uptime();
        let alive = last_good.is_some_and(|t| now.saturating_sub(t) <= SILENT_S);
        let fix = nmea.fix().filter(|_| alive);
        let gps = match (alive, fix) {
            (false, _) => Gps::Absent,
            (true, None) => Gps::Searching,
            (true, Some(_)) => Gps::Fix,
        };
        state::with(|s| {
            s.status.gps = gps;
            s.status.sats = if alive { nmea.sats } else { 0 };
            if let Some(f) = fix {
                s.status.gps_loc = Some(Location {
                    source: Source::Gps,
                    lat_e7: f.lat_e7,
                    lon_e7: f.lon_e7,
                    alt_m: f.alt_m,
                    speed_cms: f.speed_cms,
                    hdop_x10: f.hdop_x10,
                    sats: f.sats,
                    at_s: now,
                    place: Default::default(),
                });
                if let Some(t) = nmea.unix {
                    s.status.clock_base = Some(t.saturating_sub(now));
                }
            }
        });
        if gps != shown {
            match gps {
                Gps::Absent => event!("gps: no module"),
                Gps::Searching => event!("gps: searching"),
                Gps::Fix => event!("gps: fix, {} sats", nmea.sats),
            }
            shown = gps;
        }
    }
}
