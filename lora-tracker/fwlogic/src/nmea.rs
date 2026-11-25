//! NMEA 0183 parsing for the GPS module: just `RMC` and `GGA`, from any talker
//! (`GP`, `GN`, `GL`, `BD`, ...). Feed it bytes from the UART one at a time.

use heapless::Vec;

use crate::fixed::parse_scaled;
use crate::time;

/// The latest position fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fix {
    /// Degrees * 1e7.
    pub lat_e7: i32,
    /// Degrees * 1e7.
    pub lon_e7: i32,
    pub alt_m: i16,
    pub speed_cms: u16,
    pub hdop_x10: u8,
    pub sats: u8,
}

/// Accumulates sentences into the current fix and UTC time.
#[derive(Debug, Default)]
pub struct Nmea {
    line: Vec<u8, 100>,
    overflow: bool,
    fix: Fix,
    rmc_valid: bool,
    gga_valid: bool,
    /// Satellites in use, from the last GGA, even without a fix.
    pub sats: u8,
    /// UTC unix time from the last valid RMC.
    pub unix: Option<u32>,
}

impl Nmea {
    /// Feeds one byte. Returns true when it completed a sentence with a good
    /// checksum (so the module is alive, fix or not).
    pub fn feed(&mut self, b: u8) -> bool {
        match b {
            b'$' => {
                self.line.clear();
                self.overflow = false;
                let _ = self.line.push(b);
                false
            }
            b'\r' | b'\n' => {
                if self.line.is_empty() || self.overflow {
                    self.line.clear();
                    return false;
                }
                let line = core::mem::take(&mut self.line);
                core::str::from_utf8(&line).map(|s| self.sentence(s)).unwrap_or(false)
            }
            _ => {
                if !self.line.is_empty() && self.line.push(b).is_err() {
                    self.overflow = true;
                }
                false
            }
        }
    }

    /// The current fix, if both RMC and GGA say it's valid.
    pub fn fix(&self) -> Option<Fix> {
        (self.rmc_valid && self.gga_valid).then_some(self.fix)
    }

    /// Parses one complete `$...*hh` line. Returns false if it's malformed.
    fn sentence(&mut self, line: &str) -> bool {
        let Some((body, sum)) = line.strip_prefix('$').and_then(|l| l.split_once('*')) else {
            return false;
        };
        let want = u8::from_str_radix(sum.get(..2).unwrap_or(""), 16);
        if want != Ok(body.bytes().fold(0u8, |a, c| a ^ c)) {
            return false;
        }
        let mut f: Vec<&str, 24> = Vec::new();
        for part in body.split(',') {
            if f.push(part).is_err() {
                break;
            }
        }
        match f[0].get(2..) {
            Some("RMC") if f.len() >= 10 => self.rmc(&f),
            Some("GGA") if f.len() >= 10 => self.gga(&f),
            _ => {}
        }
        true
    }

    /// `$xxRMC,time,status,lat,N,lon,E,knots,course,ddmmyy,...`
    fn rmc(&mut self, f: &[&str]) {
        self.rmc_valid = f[2] == "A";
        if let (Some(lat), Some(lon)) = (coord(f[3], f[4]), coord(f[5], f[6])) {
            if self.rmc_valid {
                self.fix.lat_e7 = lat;
                self.fix.lon_e7 = lon;
            }
        }
        if let Some(knots_e3) = parse_scaled(f[7], 3) {
            self.fix.speed_cms = (knots_e3.max(0) * 514_444 / 10_000_000).min(u16::MAX as i64) as u16;
        }
        if let Some(t) = rmc_time(f[1], f[9]) {
            self.unix = Some(t);
        }
    }

    /// `$xxGGA,time,lat,N,lon,E,quality,sats,hdop,alt,M,...`
    fn gga(&mut self, f: &[&str]) {
        self.gga_valid = f[6].parse::<u8>().is_ok_and(|q| q > 0);
        self.sats = f[7].parse().unwrap_or(0);
        self.fix.sats = self.sats;
        if let Some(h) = parse_scaled(f[8], 1) {
            self.fix.hdop_x10 = h.clamp(0, 255) as u8;
        }
        if let Some(a) = parse_scaled(f[9], 0) {
            self.fix.alt_m = a.clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        }
        if let (true, Some(lat), Some(lon)) = (self.gga_valid, coord(f[2], f[3]), coord(f[4], f[5])) {
            self.fix.lat_e7 = lat;
            self.fix.lon_e7 = lon;
        }
    }
}

/// `ddmm.mmmm` (or `dddmm.mmmm`) plus hemisphere to degrees * 1e7.
fn coord(v: &str, hemi: &str) -> Option<i32> {
    let raw = parse_scaled(v, 5)?; // ddmm * 1e5
    let deg = raw / 10_000_000;
    let min_e5 = raw % 10_000_000;
    let e7 = deg * 10_000_000 + min_e5 * 100 / 60;
    match hemi {
        "N" | "E" => Some(e7 as i32),
        "S" | "W" => Some(-e7 as i32),
        _ => None,
    }
}

/// `hhmmss.ss` + `ddmmyy` to unix seconds.
fn rmc_time(t: &str, d: &str) -> Option<u32> {
    let n = |s: &str, i: usize| s.get(i..i + 2)?.parse::<u32>().ok();
    let yy = n(d, 4)? as i32;
    let year = if yy < 80 { 2000 + yy } else { 1900 + yy };
    time::unix(year, n(d, 2)?, n(d, 0)?, n(t, 0)?, n(t, 2)?, n(t, 4)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RMC: &str = "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A\r\n";
    const GGA: &str = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n";

    fn feed(n: &mut Nmea, s: &str) -> usize {
        s.bytes().filter(|b| n.feed(*b)).count()
    }

    #[test]
    fn rmc_and_gga_make_a_fix() {
        let mut n = Nmea::default();
        assert_eq!(feed(&mut n, RMC), 1);
        assert_eq!(n.fix(), None, "needs GGA too");
        assert_eq!(feed(&mut n, GGA), 1);
        let f = n.fix().unwrap();
        assert_eq!(f.lat_e7, 481_173_000);
        assert_eq!(f.lon_e7, 115_166_666);
        assert_eq!(f.alt_m, 545);
        assert_eq!(f.hdop_x10, 9);
        assert_eq!(f.sats, 8);
        assert_eq!(f.speed_cms, 1152); // 22.4 kn
        assert_eq!(n.unix, Some(764_426_119));
    }

    #[test]
    fn bad_checksum_and_noise_are_ignored() {
        let mut n = Nmea::default();
        assert_eq!(feed(&mut n, &RMC.replace("*6A", "*6B")), 0);
        assert_eq!(feed(&mut n, "garbage\r\n$GPTXT,01*00\r\n"), 0);
        assert_eq!(feed(&mut n, &"$GPGSV".repeat(30)), 0);
        assert_eq!(feed(&mut n, "\r\n"), 0);
    }

    #[test]
    fn void_rmc_drops_the_fix_but_counts_as_alive() {
        let mut n = Nmea::default();
        feed(&mut n, RMC);
        feed(&mut n, GGA);
        // Same RMC with status V; recompute the checksum.
        let body = "GPRMC,123520,V,,,,,,,230394,,";
        let sum = body.bytes().fold(0u8, |a, c| a ^ c);
        assert_eq!(feed(&mut n, &format!("${body}*{sum:02X}\r\n")), 1);
        assert_eq!(n.fix(), None);
    }

    #[test]
    fn southern_and_western_hemispheres_are_negative() {
        assert_eq!(coord("3015.000", "S"), Some(-302_500_000));
        assert_eq!(coord("09744.586", "W"), Some(-977_431_000));
        assert_eq!(coord("3015.000", "X"), None);
    }
}
