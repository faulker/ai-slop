//! WiFi: joins the first known network in range (or an open one, if the
//! tracker allows it), then while connected sets the clock over SNTP, looks
//! up a coarse IP location when there's no GPS fix, phones home with a
//! telemetry heartbeat (`POST /telemetry`, right away and then every minute),
//! and drains the outbox to `POST /ingest`.

use core::fmt::Write;

use embassy_futures::select::{Either, select};
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpAddress, Runner, Stack};
use embassy_time::{Duration, Timer, with_timeout};
use embedded_io_async::Write as _;
use esp_radio::wifi::scan::ScanConfig;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{self, AuthenticationMethod, AuthenticationMethodConfig, Interface, WifiController};
use fwlogic::location::{self, Location, Source};
use fwlogic::log::Truncate;
use fwlogic::screens::{LastConn, Link};
use fwlogic::{geoip, http, sntp, telemetry};
use heapless::String;
use tracker_proto::config::{Config, Role, WifiConfig};
use tracker_proto::telemetry::MAX_TELEMETRY;

use crate::{event, state};

/// Re-sync the clock this often while connected.
const SNTP_EVERY_S: u32 = 6 * 3600;
/// How long to wait between attempts when no network is usable.
const RETRY: Duration = Duration::from_secs(20);
/// Socket timeout for every HTTP exchange.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the embassy-net stack.
#[embassy_executor::task]
pub async fn runner(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

/// Why an HTTP or SNTP exchange failed, in words short enough for the OLED.
#[derive(Debug, Clone, Copy)]
enum NetError {
    Dns,
    Connect,
    Io,
    Timeout,
    BadReply,
}

impl NetError {
    /// One or two words for logs and the OLED.
    fn as_str(self) -> &'static str {
        match self {
            NetError::Dns => "dns failed",
            NetError::Connect => "refused",
            NetError::Io => "io error",
            NetError::Timeout => "timeout",
            NetError::BadReply => "bad reply",
        }
    }
}

/// Short text into a `LastConn` field.
fn text(args: core::fmt::Arguments<'_>) -> String<21> {
    let mut out = String::new();
    let _ = Truncate(&mut out).write_fmt(args);
    out
}

/// A network chosen from a scan.
struct Choice {
    ssid: String<32>,
    password: String<64>,
}

/// The first known network in config order that the scan saw, else (when
/// allowed) the strongest open one.
fn choose(wifi: &WifiConfig, allow_open: bool, seen: &[wifi::ap::AccessPointInfo]) -> Option<Choice> {
    for k in &wifi.known {
        if seen.iter().any(|ap| ap.ssid.as_str() == k.ssid.as_str()) {
            return Some(Choice { ssid: k.ssid.clone(), password: k.password.clone() });
        }
    }
    if !allow_open {
        return None;
    }
    seen.iter()
        .filter(|ap| matches!(ap.auth_method, Some(AuthenticationMethod::None)) && !ap.ssid.is_empty())
        .max_by_key(|ap| ap.signal_strength)
        .and_then(|ap| String::try_from(ap.ssid.as_str()).ok())
        .map(|ssid| Choice { ssid, password: String::new() })
}

/// Telemetry sequence numbers: `boot << 16 | n`, like reports, but sealed
/// in their own nonce domain so the two counters can overlap.
struct Heartbeat {
    key: [u8; 32],
    boot: u16,
    next: Option<u16>,
    last_s: Option<u32>,
    /// Whether the last heartbeat got through; logs fire only on changes.
    healthy: bool,
    /// Newest board event the server has received.
    sent_events: Option<u32>,
}

/// Connection manager. Never returns; a lost network is rejoined. `boot` is
/// `None` when the boot counter couldn't be stored, which turns telemetry off.
#[embassy_executor::task]
pub async fn task(mut ctl: WifiController<'static>, stack: Stack<'static>, cfg: &'static Config, boot: Option<u16>) {
    let wifi = cfg.wifi.as_ref().expect("wifi path enabled without [wifi]");
    let allow_open = wifi.allow_open && cfg.role == Role::Tracker;
    let server = match cfg.server.as_ref().map(|s| http::parse_url(&s.url)) {
        Some(Ok(u)) => Some(u),
        Some(Err(http::UrlError::Https)) => {
            event!("server: https unsupported, use http://");
            None
        }
        _ => {
            event!("server: bad url");
            None
        }
    };
    let (mut ingest, mut telemetry_path) = (String::<96>::new(), String::<96>::new());
    if let Some(u) = &server {
        let _ = write!(ingest, "{}/ingest", u.base);
        let _ = write!(telemetry_path, "{}/telemetry", u.base);
    }
    let mut heartbeat = match (cfg.device_key, boot) {
        (Some(key), Some(boot)) => Some(Heartbeat { key, boot, next: Some(0), last_s: None, healthy: false, sent_events: None }),
        (None, _) => {
            event!("no device_key: telemetry off");
            None
        }
        (_, None) => None,
    };

    if let Err(e) = ctl.set_config(&wifi::Config::Station(StationConfig::default())) {
        event!("wifi: start failed {e:?}");
        state::with(|s| s.status.wifi = Link::Down);
        return;
    }
    let mut last_sntp: Option<u32> = None;
    let mut last_geoip: Option<(u32, bool)> = None;
    let mut said_none = false;

    loop {
        state::with(|s| s.status.wifi = Link::Starting);
        let seen = match ctl.scan_async(&ScanConfig::default().with_max(20)).await {
            Ok(v) => v,
            Err(e) => {
                event!("wifi scan failed {e:?}");
                Timer::after(RETRY).await;
                continue;
            }
        };
        let Some(choice) = choose(wifi, allow_open, &seen) else {
            if !said_none {
                event!("wifi: no known network ({} seen)", seen.len());
                said_none = true;
            }
            state::with(|s| s.status.wifi = Link::Down);
            Timer::after(RETRY).await;
            continue;
        };
        said_none = false;
        drop(seen);

        let auth = if choice.password.is_empty() {
            AuthenticationMethodConfig::Open
        } else {
            match choice.password.as_str().try_into() {
                Ok(p) => AuthenticationMethodConfig::WpaWpa2Personal(p),
                Err(_) => {
                    event!("wifi {}: bad password", choice.ssid);
                    Timer::after(RETRY).await;
                    continue;
                }
            }
        };
        let Ok(ssid) = choice.ssid.as_str().try_into() else { continue };
        let sta = StationConfig::default().with_ssid(ssid).with_authentication(auth);
        if let Err(e) = ctl.set_config(&wifi::Config::Station(sta)) {
            event!("wifi config failed {e:?}");
            Timer::after(RETRY).await;
            continue;
        }
        if let Err(e) = ctl.connect_async().await {
            event!("wifi {} failed: {e:?}", choice.ssid);
            state::with(|s| s.status.wifi = Link::Down);
            Timer::after(RETRY).await;
            continue;
        }
        if with_timeout(Duration::from_secs(20), stack.wait_config_up()).await.is_err() {
            event!("wifi {}: no DHCP lease", choice.ssid);
            let _ = ctl.disconnect_async().await;
            Timer::after(RETRY).await;
            continue;
        }
        let ip = stack.config_v4().map(|c| c.address.address());
        let rssi = ctl.rssi().ok().map(|r| r.clamp(-128, 0) as i8);
        event!("wifi up {} {}dBm", choice.ssid, rssi.unwrap_or(0));
        let detail = match ip {
            Some(a) => text(format_args!("{}dBm {a}", rssi.unwrap_or(0))),
            None => text(format_args!("{}dBm", rssi.unwrap_or(0))),
        };
        let what = text(format_args!("{}", choice.ssid));
        let mut session_ok = 0u32;
        state::with(|s| {
            s.status.wifi = Link::Up;
            s.status.wifi_rssi = rssi;
            s.status.last_wifi = Some(LastConn {
                at_s: state::uptime(),
                what: what.clone(),
                detail: detail.clone(),
                result: text(format_args!("connected")),
            });
        });

        if let Some(hb) = heartbeat.as_mut() {
            hb.last_s = None; // phone home as soon as we're on a network
        }
        while ctl.is_connected() {
            let now = state::uptime();
            if let (Some(url), Some(hb)) = (&server, heartbeat.as_mut())
                && hb.last_s.is_none_or(|t| now.saturating_sub(t) >= telemetry::EVERY_S)
            {
                hb.last_s = Some(now);
                send_telemetry(stack, url, &telemetry_path, cfg.device_id, hb).await;
            }

            if last_sntp.is_none_or(|t| now.saturating_sub(t) >= SNTP_EVERY_S) {
                match sync_clock(stack).await {
                    Ok(unix) => {
                        last_sntp = Some(now);
                        state::with(|s| s.status.clock_base = Some(unix.saturating_sub(now)));
                        event!("clock set by sntp");
                    }
                    Err(e) => event!("sntp: {}", e.as_str()),
                }
            }

            let gps_current = state::with(|s| location::gps_current(s.status.gps_loc.as_ref(), now));
            if location::ip_lookup_due(gps_current, last_geoip, now) {
                let ok = lookup_ip_location(stack).await;
                last_geoip = Some((state::uptime(), ok));
            }

            if let Some(url) = &server
                && let Some(result) = drain(stack, url, &ingest, &mut session_ok).await
            {
                state::with(|s| {
                    if let Some(c) = s.status.last_wifi.as_mut() {
                        c.at_s = state::uptime();
                        c.result = result;
                    }
                });
            }

            if let Either::First(_) = select(ctl.wait_for_disconnect_async(), Timer::after_secs(5)).await {
                break;
            }
        }
        state::with(|s| {
            s.status.wifi = Link::Down;
            s.status.wifi_rssi = None;
        });
        event!("wifi lost {}", choice.ssid);
        Timer::after_secs(2).await;
    }
}

/// Seals a status snapshot and posts it. Logs when it starts or stops getting
/// through, so a healthy board doesn't flood the event page.
async fn send_telemetry(stack: Stack<'_>, url: &http::Url<'_>, path: &str, device_id: u32, hb: &mut Heartbeat) {
    let Some(n) = hb.next else { return };
    let (snapshot, newest) =
        state::with(|s| telemetry::from_status(&s.status, &s.log, hb.sent_events, crate::BOARD, hb.boot));
    let mut buf = [0u8; MAX_TELEMETRY];
    let seq = (hb.boot as u32) << 16 | n as u32;
    let len = match tracker_proto::telemetry::seal(&hb.key, device_id, seq, &snapshot, &mut buf) {
        Ok(len) => len,
        Err(e) => {
            event!("telemetry seal failed: {e}");
            return;
        }
    };
    // Never reuse a sequence number: when this boot's 65536 are used up, stop until reboot.
    hb.next = n.checked_add(1);
    if hb.next.is_none() {
        event!("telemetry: sequence space used up until reboot");
    }
    let head = http::request_head("POST", url.host, url.port, path, Some(len));
    let mut resp = [0u8; 256];
    let ok = match exchange(stack, url.host, url.port, head.as_bytes(), &buf[..len], &mut resp).await {
        Ok(k) => match http::status(&resp[..k]) {
            Some(200..=299) => true,
            Some(code) => {
                event!("telemetry: server said {code}");
                false
            }
            None => {
                event!("telemetry: bad reply");
                false
            }
        },
        Err(e) => {
            event!("telemetry: {} ({}:{})", e.as_str(), url.host, url.port);
            false
        }
    };
    if ok {
        hb.sent_events = newest;
        if !hb.healthy {
            event!("phoned home ({}:{})", url.host, url.port);
        }
    }
    hb.healthy = ok;
}

/// Uploads queued packets until the outbox is empty or the server stops
/// answering. Returns the line for the last-connection page, if anything was sent.
async fn drain(stack: Stack<'_>, url: &http::Url<'_>, path: &str, session_ok: &mut u32) -> Option<String<21>> {
    let mut last = None;
    loop {
        let Some(pkt) = state::with(|s| s.outbox.front().cloned()) else { return last };
        let head = http::request_head("POST", url.host, url.port, path, Some(pkt.len()));
        let mut resp = [0u8; 256];
        match exchange(stack, url.host, url.port, head.as_bytes(), &pkt, &mut resp).await {
            Ok(n) => {
                let code = http::status(&resp[..n]).unwrap_or(0);
                // 2xx and 4xx are final (see docs/protocol.md); only 5xx is retried.
                let done = matches!(code, 200..=299 | 400..=499);
                state::with(|s| {
                    if done {
                        s.outbox.pop_front();
                        s.status.queued = s.outbox.len() as u16;
                    }
                    if code / 100 == 2 {
                        s.status.sent_ok += 1;
                    } else {
                        s.status.sent_fail += 1;
                    }
                });
                if code / 100 == 2 {
                    *session_ok += 1;
                    last = Some(text(format_args!("POST {code} x{session_ok}")));
                } else {
                    event!("post: server said {code}");
                    last = Some(text(format_args!("POST {code} rejected")));
                }
                if !done {
                    return last;
                }
            }
            Err(e) => {
                state::with(|s| s.status.sent_fail += 1);
                event!("post: {}", e.as_str());
                return Some(text(format_args!("POST {}", e.as_str())));
            }
        }
    }
}

/// Looks up a coarse location from the network's public IP and stores it.
/// Returns whether it worked.
async fn lookup_ip_location(stack: Stack<'_>) -> bool {
    let head = http::request_head("GET", geoip::HOST, 80, geoip::PATH, None);
    let mut resp = [0u8; 512];
    let n = match exchange(stack, geoip::HOST, 80, head.as_bytes(), &[], &mut resp).await {
        Ok(n) => n,
        Err(e) => {
            event!("ip location: {}", e.as_str());
            return false;
        }
    };
    let resp = &resp[..n];
    let parsed = match (http::status(resp), http::body(resp)) {
        (Some(200), Some(body)) => geoip::parse(body),
        (Some(code), _) => {
            event!("ip location: HTTP {code}, {n}B");
            return false;
        }
        (None, _) => {
            event!("ip location: no reply, {n}B");
            return false;
        }
    };
    match parsed {
        Ok(g) => {
            event!("ip location {} via {}", g.city, g.ip);
            let now = state::uptime();
            state::with(|s| {
                s.status.ip_loc = Some(Location {
                    source: Source::Ip,
                    lat_e7: g.lat_e7,
                    lon_e7: g.lon_e7,
                    alt_m: 0,
                    speed_cms: 0,
                    hdop_x10: 0,
                    sats: 0,
                    at_s: now,
                    place: g.city,
                })
            });
            true
        }
        Err(geoip::GeoIpError::Failed(why)) => {
            event!("ip location failed: {why}");
            false
        }
        Err(geoip::GeoIpError::Malformed) => {
            event!("ip location: bad reply");
            false
        }
    }
}

/// Resolves a host name, or parses it if it's already an IPv4 literal.
async fn resolve(stack: Stack<'_>, host: &str) -> Result<IpAddress, NetError> {
    if let Some([a, b, c, d]) = http::ipv4(host) {
        return Ok(IpAddress::v4(a, b, c, d));
    }
    let addrs = stack.dns_query(host, DnsQueryType::A).await.map_err(|_| NetError::Dns)?;
    addrs.first().copied().ok_or(NetError::Dns)
}

/// One HTTP/1.0 exchange: connect, send head and body, read until the
/// response is complete or the server closes. Returns how many response
/// bytes landed in `resp`.
async fn exchange(
    stack: Stack<'_>,
    host: &str,
    port: u16,
    head: &[u8],
    body: &[u8],
    resp: &mut [u8],
) -> Result<usize, NetError> {
    let addr = resolve(stack, host).await?;
    let mut rx = [0u8; 1024];
    let mut tx = [0u8; 512];
    let mut sock = TcpSocket::new(stack, &mut rx, &mut tx);
    sock.set_timeout(Some(HTTP_TIMEOUT));
    sock.connect((addr, port)).await.map_err(|_| NetError::Connect)?;
    sock.write_all(head).await.map_err(|_| NetError::Io)?;
    sock.write_all(body).await.map_err(|_| NetError::Io)?;
    sock.flush().await.map_err(|_| NetError::Io)?;
    let mut n = 0;
    while n < resp.len() {
        match sock.read(&mut resp[n..]).await {
            Ok(0) => break,
            Ok(k) => {
                n += k;
                if http::complete(&resp[..n]) {
                    break;
                }
            }
            Err(embassy_net::tcp::Error::ConnectionReset) if n > 0 => break,
            // Keep a partial reply: the caller can still tell what arrived.
            Err(_) if n > 0 => break,
            Err(_) => return Err(NetError::Timeout),
        }
    }
    sock.close();
    Ok(n)
}

/// Asks an NTP server for the time.
async fn sync_clock(stack: Stack<'_>) -> Result<u32, NetError> {
    let addr = resolve(stack, sntp::HOST).await?;
    let (mut rx_meta, mut tx_meta) = ([PacketMetadata::EMPTY; 2], [PacketMetadata::EMPTY; 2]);
    let (mut rx, mut tx) = ([0u8; 128], [0u8; 128]);
    let mut sock = UdpSocket::new(stack, &mut rx_meta, &mut rx, &mut tx_meta, &mut tx);
    sock.bind(0).map_err(|_| NetError::Io)?;
    sock.send_to(&sntp::request(), (addr, 123)).await.map_err(|_| NetError::Io)?;
    let mut buf = [0u8; 64];
    let (n, _) = with_timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
        .await
        .map_err(|_| NetError::Timeout)?
        .map_err(|_| NetError::Io)?;
    sntp::parse(&buf[..n]).ok_or(NetError::BadReply)
}
