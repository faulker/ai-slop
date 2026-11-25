//! Float-free decimal parsing, shared by the NMEA and IP-location parsers.

/// Parses a decimal like `-97.7431` into an integer scaled by `10^digits`.
/// Extra fractional digits are truncated. Returns `None` on anything that
/// isn't a plain decimal number, or on overflow.
pub fn parse_scaled(s: &str, digits: u32) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let mut v: i64 = 0;
    for c in int.bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as i64)?;
    }
    let mut used = 0;
    for c in frac.bytes() {
        if !c.is_ascii_digit() {
            return None;
        }
        if used < digits {
            v = v.checked_mul(10)?.checked_add((c - b'0') as i64)?;
            used += 1;
        }
    }
    while used < digits {
        v = v.checked_mul(10)?;
        used += 1;
    }
    Some(if neg { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_pads_and_truncates() {
        assert_eq!(parse_scaled("-97.7431", 7), Some(-977_431_000));
        assert_eq!(parse_scaled("30.123456789", 7), Some(301_234_567));
        assert_eq!(parse_scaled("545", 1), Some(5450));
        assert_eq!(parse_scaled(".5", 1), Some(5));
        assert_eq!(parse_scaled("", 1), None);
        assert_eq!(parse_scaled("1a", 1), None);
        assert_eq!(parse_scaled("99999999999999999999", 0), None);
    }
}
