//! lora-tracker firmware for the Heltec WiFi LoRa 32 V2 (ESP32 + SX1276) and
//! V3 (ESP32-S3 + SX1262). The board is chosen at build time with the
//! `board-v2` / `board-v3` feature (`cargo build-v2`, `cargo build-v3`).
//!
//! Reads the config from the `ltcfg` partition, then starts only the tasks
//! its paths need (`Config::paths()`). The OLED, GPS and battery tasks always
//! run, so a board with a missing config or module still shows what's wrong.
//!
//! | | V2 | V3 |
//! |---|---|---|
//! | OLED I2C SDA / SCL / reset | 4 / 15 / 16 | 17 / 18 / 21 |
//! | Vext (low = on; OLED and Ve pins) | 21 | 36 |
//! | LoRa SCK / MOSI / MISO / NSS / reset | 5 / 27 / 19 / 18 / 14 | 9 / 10 / 11 / 8 / 12 |
//! | LoRa interrupt | DIO0 26 | DIO1 14, BUSY 13 |
//! | Battery ADC | 37 (via Vext) | 1, enable on 37 |
//! | Optional GPS: its TX to / its RX to | 23 / 22 | 47 / 48 |
//!
//! PRG button is GPIO0 on both. GPS is 9600 baud NMEA.
#![no_std]
#![no_main]

#[cfg(not(any(feature = "board-v2", feature = "board-v3")))]
compile_error!("pick a board: `cargo build-v2` (Heltec V2) or `cargo build-v3` (Heltec V3)");
#[cfg(all(feature = "board-v2", feature = "board-v3"))]
compile_error!("enable only one of `board-v2` and `board-v3`");

mod battery;
mod display;
mod gps;
mod net;
mod nolog;
mod radio;
mod report;
mod state;
mod storage;

use embassy_executor::Spawner;
use embassy_time::Timer;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::I2c;
use esp_hal::rng::Rng;
use esp_hal::spi::master::Spi;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::UartRx;
use esp_storage::FlashStorage;
use fwlogic::screens::{Link, Status};
use static_cell::StaticCell;
use tracker_proto::config::{Config, PathPlan, Role};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

/// Heltec hardware revision this build is for, reported in telemetry.
pub const BOARD: u8 = if cfg!(feature = "board-v2") { 2 } else { 3 };

static CONFIG: StaticCell<Config> = StaticCell::new();
static NET_RESOURCES: StaticCell<embassy_net::StackResources<4>> = StaticCell::new();

/// Spawns a task, panicking only if its pool is exhausted (a programming error).
macro_rules! spawn {
    ($spawner:expr, $task:expr) => {
        $spawner.spawn($task.expect("task pool exhausted"))
    };
}

#[allow(clippy::large_stack_frames, reason = "peripheral setup lives in main")]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    #[cfg(feature = "board-v2")]
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 98768);
    #[cfg(feature = "board-v3")]
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // Board wiring; see the table at the top.
    #[cfg(feature = "board-v2")]
    let (vext, sda, scl, oled_rst, gps_pin) = (p.GPIO21, p.GPIO4, p.GPIO15, p.GPIO16, p.GPIO23);
    #[cfg(feature = "board-v3")]
    let (vext, sda, scl, oled_rst, gps_pin) = (p.GPIO36, p.GPIO17, p.GPIO18, p.GPIO21, p.GPIO47);

    // Vext must stay on for the whole run: it powers the OLED (and a GPS on Ve).
    let _vext = Output::new(vext, Level::Low, OutputConfig::default());
    // Give the OLED's supply a moment to come up before talking to it.
    Timer::after_millis(50).await;

    let mut flash = FlashStorage::new(p.FLASH);
    let loaded = storage::load_config(&mut flash);
    let boot = storage::next_boot(&mut flash);

    let mut status = Status::default();
    let cfg: Option<&'static Config> = match loaded {
        Ok(c) => {
            status.role = Some(c.role);
            status.device_id = c.device_id;
            Some(CONFIG.init(c))
        }
        Err(why) => {
            status.config_error = Some(why);
            None
        }
    };
    let paths = cfg.map(|c| c.paths());
    let on = |plan: Option<PathPlan>| plan == Some(PathPlan::Enabled);
    let lora_on = on(paths.map(|p| p.lora));
    let wifi_on = on(paths.map(|p| p.wifi));
    status.lora = if lora_on { Link::Starting } else { Link::Off };
    status.wifi = if wifi_on { Link::Starting } else { Link::Off };
    let config_error = status.config_error;
    state::init(status);
    event!(
        "boot {} on {} {:?} lora:{} wifi:{}",
        boot.unwrap_or(0),
        if cfg!(feature = "board-v2") { "V2" } else { "V3" },
        cfg.map(|c| c.role),
        if lora_on { "on" } else { "off" },
        if wifi_on { "on" } else { "off" }
    );
    if let Some(why) = config_error {
        event!("config: {why}; run `lora-tracker flash-config <toml>`");
    }

    // OLED + button
    let i2c = I2c::new(p.I2C0, esp_hal::i2c::master::Config::default().with_frequency(Rate::from_khz(400)))
        .expect("i2c config")
        .with_sda(sda)
        .with_scl(scl);
    let oled_reset = Output::new(oled_rst, Level::High, OutputConfig::default());
    let button = Input::new(p.GPIO0, InputConfig::default().with_pull(Pull::Up));
    spawn!(spawner, display::task(i2c, oled_reset, button));

    // GPS (optional hardware; absence is detected at runtime)
    let gps_rx = UartRx::new(p.UART1, esp_hal::uart::Config::default().with_baudrate(9600))
        .expect("uart config")
        .with_rx(gps_pin)
        .into_async();
    spawn!(spawner, gps::task(gps_rx));

    #[cfg(feature = "board-v2")]
    spawn!(spawner, battery::task(p.ADC1, p.GPIO37, None));
    #[cfg(feature = "board-v3")]
    spawn!(spawner, battery::task(p.ADC1, p.GPIO1, Some(Output::new(p.GPIO37, Level::High, OutputConfig::default()))));

    let Some(cfg) = cfg else { idle().await };

    if let (true, Some(lora)) = (lora_on, cfg.lora.as_ref()) {
        let spi_cfg = esp_hal::spi::master::Config::default()
            .with_frequency(Rate::from_mhz(8))
            .with_mode(esp_hal::spi::Mode::_0);
        #[cfg(feature = "board-v2")]
        let pins = radio::Pins {
            spi: Spi::new(p.SPI2, spi_cfg)
                .expect("spi config")
                .with_sck(p.GPIO5)
                .with_mosi(p.GPIO27)
                .with_miso(p.GPIO19)
                .into_async(),
            nss: Output::new(p.GPIO18, Level::High, OutputConfig::default()),
            reset: Output::new(p.GPIO14, Level::High, OutputConfig::default()),
            dio0: Input::new(p.GPIO26, InputConfig::default()),
        };
        #[cfg(feature = "board-v3")]
        let pins = radio::Pins {
            spi: Spi::new(p.SPI2, spi_cfg)
                .expect("spi config")
                .with_sck(p.GPIO9)
                .with_mosi(p.GPIO10)
                .with_miso(p.GPIO11)
                .into_async(),
            nss: Output::new(p.GPIO8, Level::High, OutputConfig::default()),
            reset: Output::new(p.GPIO12, Level::High, OutputConfig::default()),
            busy: Input::new(p.GPIO13, InputConfig::default()),
            dio1: Input::new(p.GPIO14, InputConfig::default()),
        };
        match cfg.role {
            Role::Tracker => spawn!(spawner, radio::tracker(pins, lora)),
            Role::Gateway => spawn!(spawner, radio::gateway(pins, cfg, wifi_on)),
        }
    }

    if wifi_on {
        match esp_radio::wifi::WifiController::new(p.WIFI, Default::default()) {
            Ok(ctl) => {
                let rng = Rng::new();
                let seed = (rng.random() as u64) << 32 | rng.random() as u64;
                let (stack, runner) = embassy_net::new(
                    esp_radio::wifi::Interface::station(),
                    embassy_net::Config::dhcpv4(Default::default()),
                    NET_RESOURCES.init(embassy_net::StackResources::new()),
                    seed,
                );
                spawn!(spawner, net::runner(runner));
                spawn!(spawner, net::task(ctl, stack, cfg, boot));
            }
            Err(e) => {
                state::with(|s| s.status.wifi = Link::Down);
                event!("wifi init failed {e:?}");
            }
        }
    }

    if cfg.role == Role::Tracker && cfg.device_key.is_some() && (lora_on || wifi_on) {
        match boot {
            Some(boot) => spawn!(spawner, report::task(cfg, boot, lora_on, wifi_on)),
            None => event!("boot counter not writable: reports disabled"),
        }
    }

    idle().await
}

/// Keeps `main` (and the Vext pin it owns) alive while the tasks run.
async fn idle() -> ! {
    loop {
        Timer::after_secs(3600).await;
    }
}
