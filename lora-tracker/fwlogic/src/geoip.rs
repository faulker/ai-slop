//! Coarse location from the network's public IP, for boards without a GPS
//! fix. Uses [ip-api.com](https://ip-api.com)'s free endpoint, which speaks
//! plain HTTP (no TLS stack needed) and needs no key. It is city-level at
//! best, and it locates the network's ISP exit, not the board.

use heapless::String;

use crate::fixed::parse_scaled;

/// Host to connect to on port 80.
pub const HOST: &str = "ip-api.com";
/// Path asking for only the fields we read, to keep the response small.
pub const PATH: &str = "/json/?fields=status,message,lat,lon,city,query";

/// A parsed lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeoIp {
    pub lat_e7: i32,
    pub lon_e7: i32,
    pub city: String<20>,
    /// The public IP the service saw.
    pub ip: String<40>,
}

/// Why a lookup produced no location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeoIpError {
    /// The service answered `"status":"fail"`, e.g. a private or reserved IP.
    Failed(String<20>),
    /// Not the JSON we expected.
    Malformed,
}

/// Parses the JSON body of a lookup.
pub fn parse(body: &[u8]) -> Result<GeoIp, GeoIpError> {
    let json = core::str::from_utf8(body).map_err(|_| GeoIpError::Malformed)?;
    match field(json, "status") {
        Some("success") => {}
        Some(_) => return Err(GeoIpError::Failed(truncated(field(json, "message").unwrap_or("failed")))),
        None => return Err(GeoIpError::Malformed),
    }
    let deg = |k| {
        field(json, k)
            .and_then(|v| parse_scaled(v, 7))
            .and_then(|v| i32::try_from(v).ok())
            .ok_or(GeoIpError::Malformed)
    };
    Ok(GeoIp {
        lat_e7: deg("lat")?,
        lon_e7: deg("lon")?,
        city: truncated(field(json, "city").unwrap_or("")),
        ip: truncated(field(json, "query").unwrap_or("")),
    })
}

/// The raw value of top-level `"key":` in a flat JSON object: the contents
/// of a string, or the text of a number. No escapes; ip-api doesn't use them
/// for these fields.
fn field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = json;
    loop {
        let at = rest.find('"')?;
        rest = &rest[at + 1..];
        let end = rest.find('"')?;
        let (name, after) = (&rest[..end], rest[end + 1..].trim_start());
        if name == key {
            if let Some(v) = after.strip_prefix(':') {
                let v = v.trim_start();
                return match v.strip_prefix('"') {
                    Some(s) => s.find('"').map(|e| &s[..e]),
                    None => Some(v[..v.find([',', '}']).unwrap_or(v.len())].trim()),
                };
            }
        }
        rest = &rest[end + 1..];
    }
}

/// Copies as much of `s` as fits, on a char boundary.
fn truncated<const N: usize>(s: &str) -> String<N> {
    let mut out = String::new();
    for c in s.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success() {
        let body = br#"{"status":"success","city":"Austin","lat":30.2672,"lon":-97.7431,"query":"203.0.113.9"}"#;
        let g = parse(body).unwrap();
        assert_eq!((g.lat_e7, g.lon_e7), (302_672_000, -977_431_000));
        assert_eq!(g.city.as_str(), "Austin");
        assert_eq!(g.ip.as_str(), "203.0.113.9");
    }

    #[test]
    fn value_named_like_a_key_does_not_confuse_it() {
        let body = br#"{"status":"success","city":"lat","lat":1.5,"lon":2,"query":"x"}"#;
        assert_eq!(parse(body).unwrap().lat_e7, 15_000_000);
    }

    #[test]
    fn reports_failures() {
        let body = br#"{"status":"fail","message":"private range","query":"10.0.0.1"}"#;
        assert_eq!(parse(body), Err(GeoIpError::Failed(truncated("private range"))));
        assert_eq!(parse(b"<html>"), Err(GeoIpError::Malformed));
        assert_eq!(parse(br#"{"status":"success","lat":"x"}"#), Err(GeoIpError::Malformed));
    }

    #[test]
    fn long_city_is_truncated() {
        let body = br#"{"status":"success","city":"Llanfairpwllgwyngyllgogery","lat":1,"lon":2,"query":""}"#;
        assert_eq!(parse(body).unwrap().city.len(), 20);
    }
}
