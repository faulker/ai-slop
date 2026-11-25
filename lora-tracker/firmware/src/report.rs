//! Tracker report loop: every interval, seal the best known position and hand
//! it to each enabled path (LoRa directly, HTTP through the outbox).

use embassy_time::Timer;
use fwlogic::location;
use fwlogic::screens::Link;
use tracker_proto::config::Config;
use tracker_proto::packet::{self, MAX_PACKET, Report};

use crate::state::{self, Packet};
use crate::{event, radio};

/// Builds and dispatches reports forever. Only spawned on a tracker with a
/// device key and at least one of LoRa / WiFi enabled.
#[embassy_executor::task]
pub async fn task(cfg: &'static Config, boot: u16, lora: bool, http: bool) {
    let key = cfg.device_key.expect("report task needs device_key");
    // Give GPS and WiFi a moment so the first report isn't always empty.
    Timer::after_secs(10).await;
    for n in 0..=u16::MAX {
        let seq = (boot as u32) << 16 | n as u32;
        let mut r = Report { device_id: cfg.device_id, seq, ..Default::default() };
        state::with(|s| {
            let st = &s.status;
            location::fill(&mut r, location::best(st.gps_loc.as_ref(), st.ip_loc.as_ref(), st.uptime_s));
            r.ts = st.unix().unwrap_or(0);
            r.batt_mv = st.batt_mv.unwrap_or(0);
        });
        let mut buf = [0u8; MAX_PACKET];
        match packet::seal(&key, &r, &mut buf) {
            Ok(len) => {
                let pkt = Packet::from_slice(&buf[..len]).expect("sealed packet fits MAX_PACKET");
                let radio_up = lora && state::with(|s| s.status.lora == Link::Up);
                if radio_up && radio::TX.try_send((seq, pkt.clone())).is_err() {
                    event!("lora busy, skipped #{n}");
                }
                if http {
                    state::enqueue(pkt);
                }
            }
            Err(e) => event!("seal failed: {e}"),
        }
        Timer::after_secs(location::interval_s(r.flags, cfg.lora.as_ref()) as u64).await;
    }
    // 65536 reports in one boot: restart so the boot counter moves on and
    // sequence numbers (the nonce) never repeat.
    event!("sequence space used up, restarting");
    Timer::after_secs(1).await;
    esp_hal::system::software_reset();
}
