//! Battery voltage through the board's divider.
//!
//! - V2.1: GPIO37, divider powered by Vext (already on), about x3.2.
//! - V3: GPIO1, about x4.9, enabled through GPIO37. Revisions disagree on
//!   which level enables it (V3.0/3.1 low, V3.2 high), so this tries both and
//!   keeps whichever gives a plausible Li-ion reading.
//!
//! Anything implausible (no battery, USB only) shows as `--`.

use embassy_time::Timer;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::gpio::Output;
use esp_hal::peripherals::ADC1;

use crate::state;

#[cfg(feature = "board-v2")]
type Pin = esp_hal::peripherals::GPIO37<'static>;
#[cfg(feature = "board-v2")]
type Cal = esp_hal::analog::adc::AdcCalLine<ADC1<'static>>;
/// 220k over 100k.
#[cfg(feature = "board-v2")]
const DIVIDER_X100: u32 = 320;

#[cfg(feature = "board-v3")]
type Pin = esp_hal::peripherals::GPIO1<'static>;
#[cfg(feature = "board-v3")]
type Cal = esp_hal::analog::adc::AdcCalCurve<ADC1<'static>>;
/// 390k over 100k.
#[cfg(feature = "board-v3")]
const DIVIDER_X100: u32 = 490;
/// Readings outside this are treated as "no battery".
const PLAUSIBLE_MV: core::ops::RangeInclusive<u32> = 3000..=4400;

/// Samples every 30 s. `ctrl` is the divider enable pin, if the board has one.
#[embassy_executor::task]
pub async fn task(adc: ADC1<'static>, pin: Pin, mut ctrl: Option<Output<'static>>) {
    let mut cfg = AdcConfig::new();
    let mut pin = cfg.enable_pin_with_cal::<_, Cal>(pin, Attenuation::_11dB);
    let mut adc = Adc::new(adc, cfg);
    let mut enable_high = false;
    loop {
        let mut mv = None;
        for _ in 0..2 {
            if let Some(c) = ctrl.as_mut() {
                c.set_level(enable_high.into());
                Timer::after_millis(10).await;
            }
            // read_oneshot exists on every chip (read_blocking doesn't on the ESP32);
            // a conversion takes microseconds, so spinning is fine.
            let sample = loop {
                if let Ok(v) = adc.read_oneshot(&mut pin) {
                    break v;
                }
            };
            let raw = sample as u32 * DIVIDER_X100 / 100;
            if let Some(c) = ctrl.as_mut() {
                c.set_level((!enable_high).into());
            }
            if PLAUSIBLE_MV.contains(&raw) {
                mv = Some(raw as u16);
                break;
            }
            enable_high = !enable_high;
        }
        state::with(|s| s.status.batt_mv = mv);
        Timer::after_secs(30).await;
    }
}
