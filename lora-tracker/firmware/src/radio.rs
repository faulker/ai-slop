//! The onboard LoRa radio (SX1276 on the V2, SX1262 on the V3): trackers transmit sealed reports, gateways receive,
//! check, and queue them for forwarding. Radio settings are fixed here (see
//! `docs/protocol.md`); only frequency and power come from the config.

use core::fmt::Write;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Delay, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::Async;
use esp_hal::gpio::{Input, Output};
use esp_hal::spi::master::Spi;
use fwlogic::log::Truncate;
use fwlogic::screens::{LastConn, Link};
#[cfg(feature = "board-v2")]
use lora_phy::iv::GenericSx127xInterfaceVariant;
#[cfg(feature = "board-v3")]
use lora_phy::iv::GenericSx126xInterfaceVariant;
use lora_phy::mod_params::{Bandwidth, CodingRate, ModulationParams, RadioError, SpreadingFactor};
#[cfg(feature = "board-v3")]
use lora_phy::sx126x::{self, Sx126x, Sx1262, TcxoCtrlVoltage};
#[cfg(feature = "board-v2")]
use lora_phy::sx127x::{self, Sx127x, Sx1276};
use lora_phy::{LoRa, RxMode};
use tracker_proto::config::{Config, LoraConfig, MAX_PEERS};
use tracker_proto::packet::{self, ReplayGuard};

use crate::state::{self, Packet};
use crate::event;

/// Spreading factor 9 at 125 kHz keeps a ~40 byte report near 250 ms of
/// airtime, under the US 400 ms dwell limit, with good range.
const SF: SpreadingFactor = SpreadingFactor::_9;
const BW: Bandwidth = Bandwidth::_125KHz;
const CR: CodingRate = CodingRate::_4_5;
const PREAMBLE: u16 = 8;

/// Reports from the report task waiting for the radio.
pub static TX: Channel<CriticalSectionRawMutex, (u32, Packet), 2> = Channel::new();

type Bus = ExclusiveDevice<Spi<'static, Async>, Output<'static>, Delay>;
#[cfg(feature = "board-v2")]
type Radio = LoRa<Sx127x<Bus, GenericSx127xInterfaceVariant<Output<'static>, Input<'static>>, Sx1276>, Delay>;
#[cfg(feature = "board-v3")]
type Radio = LoRa<Sx126x<Bus, GenericSx126xInterfaceVariant<Output<'static>, Input<'static>>, Sx1262>, Delay>;

/// The radio's wiring (see the table in `main.rs`).
#[cfg(feature = "board-v2")]
pub struct Pins {
    pub spi: Spi<'static, Async>,
    pub nss: Output<'static>,
    pub reset: Output<'static>,
    pub dio0: Input<'static>,
}

/// The radio's wiring (see the table in `main.rs`).
#[cfg(feature = "board-v3")]
pub struct Pins {
    pub spi: Spi<'static, Async>,
    pub nss: Output<'static>,
    pub reset: Output<'static>,
    pub busy: Input<'static>,
    pub dio1: Input<'static>,
}

/// Brings the radio up, or marks LoRa down and logs why.
async fn open(pins: Pins, lora: &LoraConfig) -> Option<(Radio, ModulationParams)> {
    let result: Result<_, RadioError> = async {
        let bus = ExclusiveDevice::new(pins.spi, pins.nss, Delay).map_err(|_| RadioError::SPI)?;
        // The V2's SX1276 transmits through PA_BOOST and interrupts on DIO0.
        #[cfg(feature = "board-v2")]
        let kind = {
            let iv = GenericSx127xInterfaceVariant::new(pins.reset, pins.dio0, None, None)?;
            Sx127x::new(bus, iv, sx127x::Config { chip: Sx1276, tcxo_used: false, tx_boost: true, rx_boost: true })
        };
        // The V3 has a 1.8 V TCXO on DIO3 and uses DIO2 as the RF switch (lora-phy's default).
        #[cfg(feature = "board-v3")]
        let kind = {
            let iv = GenericSx126xInterfaceVariant::new(pins.reset, pins.dio1, pins.busy, None, None)?;
            let chip = sx126x::Config { chip: Sx1262, tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V8), use_dcdc: true, rx_boost: true };
            Sx126x::new(bus, iv, chip)
        };
        let mut radio = LoRa::new(kind, false, Delay).await?;
        let mdl = radio.create_modulation_params(SF, BW, CR, lora.frequency_hz)?;
        Ok((radio, mdl))
    }
    .await;
    match result {
        Ok(r) => {
            state::with(|s| s.status.lora = Link::Up);
            event!("lora up {}.{}MHz", lora.frequency_hz / 1_000_000, lora.frequency_hz / 100_000 % 10);
            Some(r)
        }
        Err(e) => {
            state::with(|s| s.status.lora = Link::Down);
            event!("lora: radio init failed {e:?}");
            None
        }
    }
}

/// Short text into a `LastConn` field.
fn text(args: core::fmt::Arguments<'_>) -> heapless::String<21> {
    let mut out = heapless::String::new();
    let _ = Truncate(&mut out).write_fmt(args);
    out
}

/// Tracker: sends every report handed over on [`TX`].
#[embassy_executor::task]
pub async fn tracker(pins: Pins, lora: &'static LoraConfig) {
    let Some((mut radio, mdl)) = open(pins, lora).await else { return };
    let Ok(mut params) = radio.create_tx_packet_params(PREAMBLE, false, true, false, &mdl) else {
        event!("lora: bad packet params");
        return;
    };
    loop {
        let (seq, pkt) = TX.receive().await;
        let sent = match radio.prepare_for_tx(&mdl, &mut params, lora.tx_power_dbm as i32, &pkt).await {
            Ok(()) => radio.tx().await,
            Err(e) => Err(e),
        };
        let at_s = state::uptime();
        match &sent {
            Ok(()) => event!("lora tx #{} {}B", seq & 0xFFFF, pkt.len()),
            Err(e) => event!("lora tx failed {e:?}"),
        }
        state::with(|s| {
            if sent.is_ok() {
                s.status.lora_tx += 1;
            }
            s.status.last_lora = Some(LastConn {
                at_s,
                what: text(format_args!("tx #{}", seq & 0xFFFF)),
                detail: text(format_args!("{}dBm {}B", lora.tx_power_dbm, pkt.len())),
                result: text(format_args!("{}", if sent.is_ok() { "sent (no ack)" } else { "tx error" })),
            });
        });
        let _ = radio.sleep(false).await;
    }
}

/// Gateway: listens continuously, checks packets from paired trackers, and
/// queues the original bytes for the server when WiFi is on.
#[embassy_executor::task]
pub async fn gateway(pins: Pins, cfg: &'static Config, forward: bool) {
    let lora = cfg.lora.as_ref().expect("lora path enabled without [lora]");
    let Some((mut radio, mdl)) = open(pins, lora).await else { return };
    let Ok(params) = radio.create_rx_packet_params(PREAMBLE, false, packet::MAX_PACKET as u8, true, false, &mdl) else {
        event!("lora: bad packet params");
        return;
    };
    let mut guards = [ReplayGuard::default(); MAX_PEERS];
    let mut buf = [0u8; 255];
    loop {
        if let Err(e) = radio.prepare_for_rx(RxMode::Continuous, &mdl, &params).await {
            event!("lora rx setup failed {e:?}");
            Timer::after_secs(5).await;
            continue;
        }
        let (len, st) = match radio.rx(&params, &mut buf).await {
            Ok(r) => r,
            Err(e) => {
                event!("lora rx error {e:?}");
                continue;
            }
        };
        let bytes = &buf[..len as usize];
        let verdict = check(cfg, &mut guards, bytes);
        let at_s = state::uptime();
        let keep = forward && verdict.forward;
        if keep && let Ok(pkt) = Packet::from_slice(bytes) {
            state::enqueue(pkt);
        }
        event!("lora rx {} {}dBm {}", verdict.label, st.rssi, if keep { "fwd" } else { "" });
        state::with(|s| {
            s.status.lora_rx += 1;
            if !verdict.forward {
                s.status.lora_bad += 1;
            }
            s.status.lora_rssi = Some(st.rssi);
            s.status.lora_snr = Some(st.snr);
            s.status.last_lora = Some(LastConn {
                at_s,
                what: text(format_args!("rx {len}B")),
                detail: text(format_args!("{}dBm snr {}", st.rssi, st.snr)),
                result: verdict.label,
            });
        });
    }
}

/// What the gateway makes of a received packet.
struct Verdict {
    forward: bool,
    label: heapless::String<21>,
}

/// Opens packets from paired trackers (dropping forgeries and replays) and
/// passes unknown devices through blind, as `docs/protocol.md` describes.
fn check(cfg: &Config, guards: &mut [ReplayGuard; MAX_PEERS], bytes: &[u8]) -> Verdict {
    let header = match packet::peek_header(bytes) {
        Ok(h) => h,
        Err(_) => return Verdict { forward: false, label: text(format_args!("not ours")) },
    };
    let id = header.device_id;
    let Some(peer) = cfg.peers.iter().position(|p| p.device_id == id) else {
        return Verdict { forward: true, label: text(format_args!("dev {id} blind")) };
    };
    match packet::open(&cfg.peers[peer].key, bytes) {
        Ok(r) if guards[peer].accept(r.seq).is_ok() => {
            Verdict { forward: true, label: text(format_args!("dev {id} ok")) }
        }
        Ok(_) => Verdict { forward: false, label: text(format_args!("dev {id} replay")) },
        Err(_) => Verdict { forward: false, label: text(format_args!("dev {id} bad tag")) },
    }
}
