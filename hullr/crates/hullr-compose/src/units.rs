//! Duration (`1m30s`) and byte size (`512m`) parsing for compose values.

use hullr_core::{Error, Result};

/// Parse a compose duration into seconds. Bare numbers are seconds.
pub fn parse_duration_secs(s: &str) -> Result<f64> {
    let bad = || Error::Invalid(format!("bad duration '{s}'"));
    let s = s.trim();
    if s.is_empty() {
        return Err(bad());
    }
    if let Ok(n) = s.parse::<f64>() {
        return Ok(n);
    }
    let mut total = 0.0;
    let mut rest = s;
    while !rest.is_empty() {
        let num_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .ok_or_else(bad)?;
        if num_end == 0 {
            return Err(bad());
        }
        let n: f64 = rest[..num_end].parse().map_err(|_| bad())?;
        rest = &rest[num_end..];
        let unit_end = rest
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(rest.len());
        let mult = match &rest[..unit_end] {
            "ns" => 1e-9,
            "us" | "µs" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => return Err(bad()),
        };
        total += n * mult;
        rest = &rest[unit_end..];
    }
    Ok(total)
}

/// Whole seconds, rounding any fraction up (so `500ms` is 1).
pub fn duration_secs_ceil(s: &str) -> Result<u64> {
    Ok(parse_duration_secs(s)?.ceil() as u64)
}

/// Parse a size like `512m`, `1g`, `1.5GB`, `2048` (bytes). Units are 1024-based.
pub fn parse_bytes(s: &str) -> Result<u64> {
    let bad = || Error::Invalid(format!("bad memory size '{s}'"));
    let t = s.trim().to_ascii_lowercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let n: f64 = t[..split].parse().map_err(|_| bad())?;
    let mult: f64 = match t[split..].trim() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024.0_f64.powi(4),
        _ => return Err(bad()),
    };
    Ok((n * mult) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration_secs("10s").unwrap(), 10.0);
        assert_eq!(parse_duration_secs("1m30s").unwrap(), 90.0);
        assert_eq!(parse_duration_secs("1h").unwrap(), 3600.0);
        assert_eq!(parse_duration_secs("500ms").unwrap(), 0.5);
        assert_eq!(parse_duration_secs("1.5s").unwrap(), 1.5);
        assert_eq!(parse_duration_secs("30").unwrap(), 30.0);
        assert_eq!(duration_secs_ceil("500ms").unwrap(), 1);
        assert_eq!(duration_secs_ceil("0s").unwrap(), 0);
        assert!(parse_duration_secs("abc").is_err());
        assert!(parse_duration_secs("10x").is_err());
        assert!(parse_duration_secs("").is_err());
    }

    #[test]
    fn bytes() {
        assert_eq!(parse_bytes("512m").unwrap(), 512 * 1024 * 1024);
        assert_eq!(parse_bytes("1g").unwrap(), 1 << 30);
        assert_eq!(parse_bytes("1.5GB").unwrap(), 1610612736);
        assert_eq!(parse_bytes("2048").unwrap(), 2048);
        assert_eq!(parse_bytes("64kb").unwrap(), 65536);
        assert!(parse_bytes("12q").is_err());
        assert!(parse_bytes("m").is_err());
    }
}
