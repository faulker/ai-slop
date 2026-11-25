//! HTTP routes: packet ingest for boards, JSON API and map page for people.
//! The map and API are open to anyone who can reach the port.
//!
//! - `POST /ingest`: body is one sealed packet (raw bytes). The AEAD tag is the
//!   authentication.
//! - `POST /telemetry`: body is one sealed telemetry heartbeat, same rules.
//! - `GET /`: the map page. `GET /log`: live log and connected devices.
//! - `GET /api/devices`: every configured tracker with its latest report,
//!   last contact and telemetry.
//! - `GET /api/devices/{id}/track?hours=24&limit=2000`: recent reports, oldest first.
//! - `GET /api/events`: server-sent events, the recent log then live lines.
//!
//! Every board request, accepted or not, is logged (see [`crate::live`]).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use futures_util::stream::{self, Stream, StreamExt};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tracker_proto::packet::{self, Header, MAX_PACKET};
use tracker_proto::telemetry::{self, MAX_TELEMETRY};
use tracker_proto::Error as ProtoError;

use crate::config::{Config, Device};
use crate::db::{self, Insert, StoredReport};
use crate::live::{Level, Live, Seen, TelemetryView, ONLINE_S};

const MAP_HTML: &str = include_str!("map.html");
const LOG_HTML: &str = include_str!("log.html");

/// Shared by every handler.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: Arc<Mutex<Connection>>,
    pub live: Arc<Live>,
}

/// Builds the full router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/ingest", post(ingest).layer(DefaultBodyLimit::max(MAX_PACKET)))
        .route("/telemetry", post(telemetry_in).layer(DefaultBodyLimit::max(MAX_TELEMETRY)))
        .route("/", get(map_page))
        .route("/log", get(log_page))
        .route("/api/devices", get(devices))
        .route("/api/devices/{id}/track", get(track))
        .route("/api/events", get(events))
        .with_state(state)
}

/// The peer address, when the server runs with connect info (tests don't).
type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

/// `ip:port` of the peer, or `-`.
fn peer(p: Peer) -> String {
    p.map_or_else(|| "-".into(), |Extension(ConnectInfo(a))| a.to_string())
}

/// Finds the configured device for a sealed packet's header, logging and
/// returning the error response when it can't.
fn device_for<'a>(
    st: &'a AppState,
    addr: &str,
    what: &str,
    header: Result<Header, ProtoError>,
) -> Result<(Header, &'a Device), (StatusCode, String)> {
    let header = header.map_err(|e| {
        st.live.log(now(), Level::Warn, addr, None, None, format!("{what} rejected: {e}"));
        (StatusCode::BAD_REQUEST, format!("{e}\n"))
    })?;
    let Some(device) = st.cfg.devices.get(&header.device_id) else {
        st.live.log(
            now(),
            Level::Warn,
            addr,
            Some(header.device_id),
            None,
            format!("{what} rejected: unknown device (add it to server.toml with `lora-tracker pair`)"),
        );
        return Err((StatusCode::FORBIDDEN, "unknown device\n".into()));
    };
    Ok((header, device))
}

/// Logs a packet that failed to open and builds the matching response.
fn open_failed(st: &AppState, addr: &str, what: &str, h: &Header, d: &Device, e: ProtoError) -> Response {
    let (code, why) = match e {
        ProtoError::AuthFailed => (StatusCode::FORBIDDEN, "authentication failed (key mismatch?)".to_string()),
        e => (StatusCode::BAD_REQUEST, e.to_string()),
    };
    st.live.log(now(), Level::Warn, addr, Some(h.device_id), Some(&d.name), format!("{what} rejected: {why}"));
    (code, format!("{why}\n")).into_response()
}

/// Current unix time in seconds.
fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Opens and stores one sealed packet.
///
/// 204 when stored (or already stored, so a board can safely retry), 400 for a
/// malformed packet, 403 for an unknown device or a failed tag. Boards should
/// drop a packet on 4xx and keep it queued on 5xx or no answer.
async fn ingest(State(st): State<AppState>, p: Peer, body: Bytes) -> Response {
    let addr = peer(p);
    let (header, device) = match device_for(&st, &addr, "report", packet::peek_header(&body)) {
        Ok(x) => x,
        Err(resp) => return resp.into_response(),
    };
    let report = match packet::open(&device.key, &body) {
        Ok(r) => r,
        Err(e) => return open_failed(&st, &addr, "report", &header, device, e),
    };
    let at = now();
    st.live.saw(header.device_id, at, &addr, None);
    let result = db::insert(&st.db.lock().unwrap(), &report, at);
    match result {
        Ok(stored) => {
            let what = if stored == Insert::Duplicate { "duplicate report (already stored)" } else { "report" };
            let pos = match (report.flags & packet::FLAG_NO_FIX != 0, report.flags & packet::FLAG_APPROX != 0) {
                (true, _) => "no fix".to_string(),
                (false, approx) => format!(
                    "{:.5},{:.5}{}",
                    report.lat_e7 as f64 / 1e7,
                    report.lon_e7 as f64 / 1e7,
                    if approx { " (approx, from IP)" } else { "" }
                ),
            };
            let text = format!("{what} boot {} #{}: {pos}", report.seq >> 16, report.seq & 0xFFFF);
            st.live.log(at, Level::Info, &addr, Some(header.device_id), Some(&device.name), text);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            let text = format!("storing report failed: {e:#}");
            st.live.log(at, Level::Warn, &addr, Some(header.device_id), Some(&device.name), text);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Opens one telemetry heartbeat and records it as the device's live state.
/// Same status codes as [`ingest`]. Telemetry is kept in memory only.
async fn telemetry_in(State(st): State<AppState>, p: Peer, body: Bytes) -> Response {
    let addr = peer(p);
    let (header, device) = match device_for(&st, &addr, "telemetry", telemetry::peek_header(&body)) {
        Ok(x) => x,
        Err(resp) => return resp.into_response(),
    };
    let t = match telemetry::open(&device.key, &body) {
        Ok((_, t)) => t,
        Err(e) => return open_failed(&st, &addr, "telemetry", &header, device, e),
    };
    if !st.live.accept_telemetry(header.device_id, header.seq) {
        let text = format!("telemetry rejected: replayed or old seq {}", header.seq);
        st.live.log(now(), Level::Warn, &addr, Some(header.device_id), Some(&device.name), text);
        return (StatusCode::CONFLICT, "replayed telemetry\n").into_response();
    }
    let at = now();
    for e in &t.events {
        let text = format!("board: {}", e.text);
        st.live.log(at - e.age_s as i64, Level::Info, &addr, Some(header.device_id), Some(&device.name), text);
    }
    let view = TelemetryView::from(&t);
    let first = st.live.seen(header.device_id).is_none_or(|s| at - s.last_seen > ONLINE_S);
    let text = format!("{}{}", if first { "connected: " } else { "telemetry: " }, view.summary());
    st.live.saw(header.device_id, at, &addr, Some(view));
    st.live.log(at, Level::Info, &addr, Some(header.device_id), Some(&device.name), text);
    StatusCode::NO_CONTENT.into_response()
}

/// Serves the live log page.
async fn log_page() -> Html<&'static str> {
    Html(LOG_HTML)
}

/// Streams the kept log, then new lines as they happen, as server-sent events
/// (one JSON [`crate::live::LogEvent`] per message).
async fn events(State(st): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe before copying the backlog so nothing falls in the gap.
    let live = BroadcastStream::new(st.live.subscribe()).filter_map(|r| async move { r.ok() });
    let all = stream::iter(st.live.recent()).chain(live);
    let sse = all.map(|ev| Ok(Event::default().json_data(&ev).unwrap_or_default()));
    Sse::new(sse).keep_alive(KeepAlive::default())
}

/// Serves the map page.
async fn map_page() -> Html<&'static str> {
    Html(MAP_HTML)
}

/// One entry in `GET /api/devices`.
#[derive(Serialize)]
struct DeviceSummary {
    device_id: u32,
    name: String,
    /// False for a device that has stored reports but is no longer in server.toml.
    configured: bool,
    /// Newest report of any kind: when the device was last heard from.
    latest: Option<StoredReport>,
    /// True when the device was heard from (report or telemetry) recently.
    online: bool,
    /// Last contact and latest telemetry since the server started.
    seen: Option<Seen>,
    /// Newest report with a current position (GPS, or an IP estimate flagged
    /// `approx`): the last known location.
    last_fix: Option<StoredReport>,
}

/// Lists every configured tracker plus any device with stored reports, sorted by id.
async fn devices(State(st): State<AppState>) -> Result<Json<Vec<DeviceSummary>>, ApiError> {
    let conn = st.db.lock().unwrap();
    let mut ids: Vec<u32> = st.cfg.devices.keys().copied().chain(db::reported_ids(&conn)?).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out = Vec::with_capacity(ids.len());
    for device_id in ids {
        let configured = st.cfg.devices.get(&device_id);
        out.push(DeviceSummary {
            device_id,
            name: configured.map_or_else(|| format!("Tracker {device_id}"), |d| d.name.clone()),
            configured: configured.is_some(),
            online: st.live.seen(device_id).is_some_and(|s| now() - s.last_seen <= ONLINE_S),
            seen: st.live.seen(device_id),
            latest: db::latest(&conn, device_id)?,
            last_fix: db::latest_fix(&conn, device_id)?,
        });
    }
    Ok(Json(out))
}

/// Query for `GET /api/devices/{id}/track`.
#[derive(Deserialize)]
struct TrackQuery {
    hours: Option<u32>,
    limit: Option<u32>,
}

/// Recent reports for one tracker, oldest first.
async fn track(
    State(st): State<AppState>,
    Path(id): Path<u32>,
    Query(q): Query<TrackQuery>,
) -> Result<Json<Vec<StoredReport>>, ApiError> {
    let conn = st.db.lock().unwrap();
    if !st.cfg.devices.contains_key(&id) && db::latest(&conn, id)?.is_none() {
        return Err(ApiError::NotFound);
    }
    let hours = q.hours.unwrap_or(24).min(24 * 366);
    let limit = q.limit.unwrap_or(2000).clamp(1, 10_000);
    let since = now() - hours as i64 * 3600;
    Ok(Json(db::track(&conn, id, since, limit)?))
}

/// Errors from the JSON API.
enum ApiError {
    NotFound,
    Db(anyhow::Error),
}

impl From<anyhow::Error> for ApiError {
    /// Wraps a database failure.
    fn from(e: anyhow::Error) -> Self {
        ApiError::Db(e)
    }
}

impl IntoResponse for ApiError {
    /// 404 for an unknown device, 500 (logged) for database failures.
    fn into_response(self) -> Response {
        match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "unknown device\n").into_response(),
            ApiError::Db(e) => {
                eprintln!("api: {e:#}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use tracker_proto::packet::Report;

    const KEY: [u8; 32] = [7; 32];

    /// State with device 1 configured and an in-memory database.
    fn state() -> AppState {
        let text = format!("[[device]]\ndevice_id = 1\nname = \"Truck\"\nkey = \"{}\"\n", hex::encode(KEY));
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        AppState {
            cfg: Arc::new(config::parse(&text).unwrap()),
            db: Arc::new(Mutex::new(conn)),
            live: Arc::new(Live::new(false)),
        }
    }

    /// A router over [`state`].
    fn app() -> Router {
        router(state())
    }

    /// Seals a telemetry heartbeat with no position (a board without GPS).
    fn sealed_telemetry(key: &[u8; 32], device_id: u32, seq: u32) -> Vec<u8> {
        use tracker_proto::config::Role;
        use tracker_proto::telemetry::{BoardEvent, Gps, Telemetry};
        let t = Telemetry {
            role: Role::Tracker,
            board: 2,
            ts: 0,
            uptime_s: 30,
            boot: 8,
            batt_mv: None,
            gps: Gps::Absent,
            sats: 0,
            wifi_rssi: Some(-52),
            lora_tx: 1,
            lora_rx: 0,
            lora_bad: 0,
            lora_rssi: None,
            sent_ok: 0,
            sent_fail: 0,
            queued: 1,
            position: None,
            events: [BoardEvent { age_s: 5, text: "ip location: timeout".try_into().unwrap() }].into_iter().collect(),
        };
        let mut buf = [0u8; MAX_TELEMETRY];
        let n = telemetry::seal(key, device_id, seq, &t, &mut buf).unwrap();
        buf[..n].to_vec()
    }

    /// POSTs raw bytes to `path`.
    fn post_to(path: &str, bytes: Vec<u8>) -> axum::http::Request<Body> {
        axum::http::Request::post(path).body(Body::from(bytes)).unwrap()
    }

    /// Seals a report for `device_id` with `key`.
    fn sealed(key: &[u8; 32], device_id: u32, seq: u32) -> Vec<u8> {
        let r = Report { device_id, seq, ts: 1, lat_e7: 400_000_000, lon_e7: -1_050_000_000, ..Default::default() };
        let mut buf = [0u8; MAX_PACKET];
        let n = packet::seal(key, &r, &mut buf).unwrap();
        buf[..n].to_vec()
    }

    /// Sends a request and returns status and body text.
    async fn send(app: &Router, req: axum::http::Request<Body>) -> (StatusCode, String) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    /// POSTs raw bytes to `/ingest`.
    fn post_packet(bytes: Vec<u8>) -> axum::http::Request<Body> {
        axum::http::Request::post("/ingest").body(Body::from(bytes)).unwrap()
    }

    /// GETs a path.
    fn get_req(uri: &str) -> axum::http::Request<Body> {
        axum::http::Request::get(uri).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn ingest_stores_and_api_returns_it() {
        let app = app();
        assert_eq!(send(&app, post_packet(sealed(&KEY, 1, 9))).await.0, StatusCode::NO_CONTENT);
        let (status, body) = send(&app, get_req("/api/devices")).await;
        assert_eq!(status, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v[0]["name"], "Truck");
        assert_eq!(v[0]["latest"]["seq"], 9);
        assert_eq!(v[0]["latest"]["lat"], 40.0);

        let (_, body) = send(&app, get_req("/api/devices/1/track")).await;
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap().as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn lists_unconfigured_devices_and_last_fix_ignores_no_fix_reports() {
        let conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        // Device 5 reported in the past but has since been removed from server.toml.
        let fix = Report { device_id: 5, seq: 1, lat_e7: 400_000_000, ..Default::default() };
        db::insert(&conn, &fix, 10).unwrap();
        db::insert(&conn, &Report { seq: 2, lat_e7: 0, flags: packet::FLAG_NO_FIX, ..fix }, 20).unwrap();
        let text = format!("[[device]]\ndevice_id = 1\nkey = \"{}\"\n", hex::encode(KEY));
        let app = router(AppState {
            cfg: Arc::new(config::parse(&text).unwrap()),
            db: Arc::new(Mutex::new(conn)),
            live: Arc::new(Live::new(false)),
        });

        let (_, body) = send(&app, get_req("/api/devices")).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        assert_eq!(v[0]["device_id"], 1);
        assert!(v[0]["last_fix"].is_null());
        assert_eq!(v[1]["name"], "Tracker 5");
        assert_eq!(v[1]["configured"], false);
        assert_eq!(v[1]["latest"]["seq"], 2);
        assert_eq!(v[1]["last_fix"]["seq"], 1);
        assert_eq!(v[1]["last_fix"]["lat"], 40.0);
        assert_eq!(send(&app, get_req("/api/devices/5/track?hours=1000000")).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn retried_packet_is_accepted_without_a_second_row() {
        let app = app();
        let p = sealed(&KEY, 1, 4);
        assert_eq!(send(&app, post_packet(p.clone())).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, post_packet(p)).await.0, StatusCode::NO_CONTENT);
        let (_, body) = send(&app, get_req("/api/devices/1/track")).await;
        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap().as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejects_bad_packets() {
        let app = app();
        assert_eq!(send(&app, post_packet(vec![1, 2, 3])).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(send(&app, post_packet(sealed(&KEY, 2, 1))).await.0, StatusCode::FORBIDDEN);
        assert_eq!(send(&app, post_packet(sealed(&[8; 32], 1, 1))).await.0, StatusCode::FORBIDDEN);
        let mut tampered = sealed(&KEY, 1, 1);
        tampered[12] ^= 1;
        assert_eq!(send(&app, post_packet(tampered)).await.0, StatusCode::FORBIDDEN);
        assert_eq!(send(&app, post_packet(vec![0; MAX_PACKET + 1])).await.0, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(send(&app, get_req("/api/devices/2/track")).await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn telemetry_without_a_position_marks_the_device_online() {
        let st = state();
        let app = router(st.clone());
        let (_, body) = send(&app, get_req("/api/devices")).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v[0]["online"], false);

        let (code, _) = send(&app, post_to("/telemetry", sealed_telemetry(&KEY, 1, 8 << 16))).await;
        assert_eq!(code, StatusCode::NO_CONTENT);
        let (_, body) = send(&app, get_req("/api/devices")).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v[0]["online"], true);
        let t = &v[0]["seen"]["telemetry"];
        assert_eq!((t["board"].as_str(), t["gps"].as_str(), t["wifi_rssi"].as_i64()), (Some("Heltec V2"), Some("none"), Some(-52)));
        assert!(t["position"].is_null());
        let recent = st.live.recent();
        assert_eq!(recent[0].text, "board: ip location: timeout");
        assert!(recent[1].text.starts_with("connected: Heltec V2 tracker"));
    }

    #[tokio::test]
    async fn replayed_telemetry_is_rejected() {
        let st = state();
        let app = router(st.clone());
        let hb = sealed_telemetry(&KEY, 1, 8 << 16 | 5);
        assert_eq!(send(&app, post_to("/telemetry", hb.clone())).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, post_to("/telemetry", hb)).await.0, StatusCode::CONFLICT);
        let older = sealed_telemetry(&KEY, 1, 8 << 16 | 4);
        assert_eq!(send(&app, post_to("/telemetry", older)).await.0, StatusCode::CONFLICT);
        // After a reboot the boot counter moves on, so seq is higher again.
        let next_boot = sealed_telemetry(&KEY, 1, 9 << 16);
        assert_eq!(send(&app, post_to("/telemetry", next_boot)).await.0, StatusCode::NO_CONTENT);
        assert!(st.live.recent().iter().any(|e| e.level == Level::Warn && e.text.contains("replayed")));
    }

    #[tokio::test]
    async fn rejected_board_requests_are_logged_with_a_reason() {
        let st = state();
        let app = router(st.clone());
        assert_eq!(send(&app, post_to("/telemetry", sealed_telemetry(&KEY, 9, 1))).await.0, StatusCode::FORBIDDEN);
        assert_eq!(send(&app, post_to("/telemetry", sealed_telemetry(&[8; 32], 1, 1))).await.0, StatusCode::FORBIDDEN);
        // A report posted to /telemetry is the wrong format, not a forgery.
        assert_eq!(send(&app, post_to("/telemetry", sealed(&KEY, 1, 1))).await.0, StatusCode::BAD_REQUEST);
        let texts: Vec<String> = st.live.recent().into_iter().filter(|e| e.level == Level::Warn).map(|e| e.text).collect();
        assert!(texts[0].contains("unknown device"), "{texts:?}");
        assert!(texts[1].contains("authentication failed"), "{texts:?}");
        assert!(texts[2].contains("unsupported version 1"), "{texts:?}");
    }

    #[tokio::test]
    async fn stored_reports_are_logged_and_streamed() {
        let st = state();
        let app = router(st.clone());
        assert_eq!(send(&app, post_packet(sealed(&KEY, 1, 3 << 16 | 4))).await.0, StatusCode::NO_CONTENT);
        assert_eq!(st.live.recent()[0].text, "report boot 3 #4: 40.00000,-105.00000");

        let resp = app.clone().oneshot(get_req("/api/events")).await.unwrap();
        assert_eq!(resp.headers()["content-type"], "text/event-stream");
        let frame = resp.into_body().frame().await.unwrap().unwrap().into_data().unwrap();
        let text = String::from_utf8_lossy(&frame);
        assert!(text.starts_with("data: ") && text.contains("report boot 3 #4"), "{text}");
    }

}
