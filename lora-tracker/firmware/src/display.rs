//! The onboard SSD1306 OLED and the PRG button.
//!
//! The PRG button steps through the pages (stats, location, last connection,
//! events). With no press for a minute the pages rotate on their own.

use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use esp_hal::Blocking;
use esp_hal::gpio::{Input, Output};
use esp_hal::i2c::master::I2c;
use fwlogic::screens::{self, Page};
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};

use crate::{event, state};

/// How long a button press holds the page before auto-rotation resumes.
const HOLD: Duration = Duration::from_secs(60);
/// Time per page while rotating.
const ROTATE: Duration = Duration::from_secs(8);
/// Redraw period (uptime and ages tick).
const REFRESH: Duration = Duration::from_millis(500);

/// Drives the OLED forever. If the panel doesn't answer, logs it and exits;
/// everything else keeps running.
#[embassy_executor::task]
pub async fn task(i2c: I2c<'static, Blocking>, mut reset: Output<'static>, mut button: Input<'static>) {
    reset.set_low();
    Timer::after_millis(10).await;
    reset.set_high();
    Timer::after_millis(10).await;

    let mut oled = Ssd1306::new(I2CDisplayInterface::new(i2c), DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    if oled.init().is_err() {
        event!("oled: no answer on I2C");
        return;
    }
    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    let mut page = Page::default();
    let mut pressed_at: Option<Instant> = None;
    let mut shown_at = Instant::now();
    loop {
        let lines = state::with(|s| screens::render(page, &s.status, &s.log));
        oled.clear_buffer();
        for (i, l) in lines.iter().enumerate() {
            let _ = Text::with_baseline(l, Point::new(0, i as i32 * 10 + 2), style, Baseline::Top).draw(&mut oled);
        }
        if oled.flush().is_err() {
            esp_println::println!("oled: flush failed");
        }

        if let Either::First(()) = select(button.wait_for_falling_edge(), Timer::after(REFRESH)).await {
            page = page.next();
            pressed_at = Some(Instant::now());
            shown_at = Instant::now();
            Timer::after_millis(150).await; // debounce
            continue;
        }
        let idle = pressed_at.is_none_or(|t| t.elapsed() >= HOLD);
        if idle && shown_at.elapsed() >= ROTATE {
            page = page.next();
            shown_at = Instant::now();
        }
    }
}
