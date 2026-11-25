//! Calendar math without a clock library.

/// Unix seconds for a UTC date and time, or `None` if any part is out of range.
pub fn unix(year: i32, month: u32, day: u32, h: u32, m: u32, s: u32) -> Option<u32> {
    if !(1970..=2105).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if h > 23 || m > 59 || s > 60 {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u32::try_from(days * 86_400 + (h * 3600 + m * 60 + s) as i64).ok()
}

/// `HH:MM` (UTC) of a unix time.
pub fn hh_mm(unix: u32) -> (u32, u32) {
    let day_s = unix % 86_400;
    (day_s / 3600, day_s % 3600 / 60)
}

/// Compact age like `12s`, `5m`, `3h`, `2d`, for tight OLED lines.
pub fn age(secs: u32) -> heapless::String<5> {
    use core::fmt::Write;
    let mut out = heapless::String::new();
    let _ = match secs {
        0..=59 => write!(out, "{secs}s"),
        60..=3599 => write!(out, "{}m", secs / 60),
        3600..=86_399 => write!(out, "{}h", secs / 3600),
        _ => write!(out, "{}d", (secs / 86_400).min(999)),
    };
    out
}

/// Uptime like `3d04h`, `1h02m` or `5m09s`.
pub fn uptime(secs: u32) -> heapless::String<8> {
    use core::fmt::Write;
    let mut out = heapless::String::new();
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let _ = if d > 0 {
        write!(out, "{}d{h:02}h", d.min(999))
    } else if h > 0 {
        write!(out, "{h}h{m:02}m")
    } else {
        write!(out, "{m}m{s:02}s")
    };
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_matches_known_dates() {
        assert_eq!(unix(1970, 1, 1, 0, 0, 0), Some(0));
        assert_eq!(unix(1994, 3, 23, 12, 35, 19), Some(764_426_119));
        assert_eq!(unix(2024, 2, 29, 0, 0, 0), Some(1_709_164_800));
        assert_eq!(unix(2024, 13, 1, 0, 0, 0), None);
    }

    #[test]
    fn formats_ages_and_uptime() {
        assert_eq!(age(9).as_str(), "9s");
        assert_eq!(age(125).as_str(), "2m");
        assert_eq!(age(7200).as_str(), "2h");
        assert_eq!(age(200_000).as_str(), "2d");
        assert_eq!(uptime(309).as_str(), "5m09s");
        assert_eq!(uptime(3720).as_str(), "1h02m");
        assert_eq!(uptime(273_600).as_str(), "3d04h");
        assert_eq!(hh_mm(764_426_119), (12, 35));
    }
}
