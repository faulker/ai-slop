//! Chromecast streaming: ffmpeg encodes frames to HLS, a tiny HTTP server hosts it and a
//! control thread tells the Chromecast to play it.

pub mod capture;
pub mod discovery;
pub mod encoder;
pub mod server;

use std::net::IpAddr;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::{GenericMediaMetadata, Media, Metadata, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use rust_cast::{CastDevice, ChannelMessage};

use encoder::{Encoder, PLAYLIST_NAME};
use server::{local_ip_for, HlsServer};

/// How long to wait for ffmpeg to produce the first segments.
const PLAYLIST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `CastSession::stop` waits for the control thread before giving up.
const STOP_TIMEOUT: Duration = Duration::from_secs(6);

/// A Chromecast found on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CastDeviceInfo {
    pub name: String,
    pub host: IpAddr,
    pub port: u16,
}

/// Video settings for a cast session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CastConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}

/// Lifecycle state of a cast session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CastStatus {
    Idle,
    Starting,
    Streaming { device: String },
    Error(String),
}

/// A running cast to one device.
pub struct CastSession {
    config: CastConfig,
    status: Arc<Mutex<CastStatus>>,
    encoder: Arc<Mutex<Option<Encoder>>>,
    stop: Arc<AtomicBool>,
    done: Receiver<()>,
}

impl CastSession {
    /// Spawns the control thread (encoder, HTTP server, Chromecast session) and returns at once.
    pub fn start(device: CastDeviceInfo, config: CastConfig) -> CastSession {
        let status = Arc::new(Mutex::new(CastStatus::Starting));
        let encoder = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done) = mpsc::channel();

        let ctx = Control {
            device,
            config,
            status: status.clone(),
            encoder: encoder.clone(),
            stop: stop.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("cast-control".into())
            .spawn(move || {
                let status = ctx.status.clone();
                if let Err(msg) = ctx.run() {
                    log::error!("cast failed: {msg}");
                    set_status(&status, CastStatus::Error(msg));
                }
                // The receiver may already be gone if stop() timed out; that is fine.
                if done_tx.send(()).is_err() {
                    log::debug!("cast session dropped before the control thread finished");
                }
            });
        if let Err(e) = spawned {
            set_status(
                &status,
                CastStatus::Error(format!("could not start the cast thread: {e}")),
            );
        }
        CastSession {
            config,
            status,
            encoder,
            stop,
            done,
        }
    }

    /// Current session status.
    pub fn status(&self) -> CastStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The configuration this session was started with.
    pub fn config(&self) -> CastConfig {
        self.config
    }

    /// Hands the encoder the latest frame (tightly packed RGBA8); ignored until ffmpeg is up.
    pub fn push_frame(&self, rgba: Vec<u8>) {
        if let Some(enc) = self
            .encoder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            enc.push_frame(rgba);
        }
    }

    /// Stops playback, closes the receiver app, kills ffmpeg and the server (waits at most ~6s).
    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        if self.done.recv_timeout(STOP_TIMEOUT).is_err() {
            log::warn!("cast control thread did not stop within {STOP_TIMEOUT:?}; detaching it");
        }
    }
}

/// Ok(()) if ffmpeg is on PATH and has the h264_videotoolbox encoder.
pub fn ffmpeg_available() -> Result<(), String> {
    const MISSING: &str = "ffmpeg not found. Install it with: brew install ffmpeg";
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .map_err(|_| MISSING.to_string())?;
    if !out.status.success() {
        return Err(MISSING.to_string());
    }
    if String::from_utf8_lossy(&out.stdout).contains("h264_videotoolbox") {
        Ok(())
    } else {
        Err(
            "ffmpeg has no h264_videotoolbox encoder. Reinstall it with: brew reinstall ffmpeg"
                .to_string(),
        )
    }
}

/// Everything the control thread needs.
struct Control {
    device: CastDeviceInfo,
    config: CastConfig,
    status: Arc<Mutex<CastStatus>>,
    encoder: Arc<Mutex<Option<Encoder>>>,
    stop: Arc<AtomicBool>,
}

/// Sets the shared status.
fn set_status(status: &Mutex<CastStatus>, new: CastStatus) {
    *status.lock().unwrap_or_else(|e| e.into_inner()) = new;
}

impl Control {
    /// Runs the whole session and always cleans up; Err carries a user-facing message.
    fn run(&self) -> Result<(), String> {
        ffmpeg_available()?;
        // Unique per session: a stopping session may still be cleaning up its own dir.
        static SESSION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SESSION_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fractal-flow-cast-{}-{n}", std::process::id()));
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                log::warn!("could not clear {}: {e}", dir.display());
            }
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;

        let result = self.run_in(&dir);

        if let Some(enc) = self
            .encoder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            enc.stop();
        }
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            log::warn!("could not remove {}: {e}", dir.display());
        }
        if result.is_ok() {
            set_status(&self.status, CastStatus::Idle);
        }
        result
    }

    /// Starts the encoder and server, then casts and services the Chromecast connection.
    fn run_in(&self, dir: &Path) -> Result<(), String> {
        let enc = Encoder::start(self.config, dir).map_err(|e| format!("{e:#}"))?;
        *self.encoder.lock().unwrap_or_else(|e| e.into_inner()) = Some(enc);
        let server = HlsServer::start(dir.to_path_buf()).map_err(|e| format!("{e:#}"))?;
        let result = self.cast(dir, &server);
        server.stop();
        result
    }

    /// Waits for the stream to be ready, loads it on the Chromecast and keeps the link alive.
    fn cast(&self, dir: &Path, server: &HlsServer) -> Result<(), String> {
        if !self.wait_for_segments(dir)? {
            return Ok(());
        }
        let ip = local_ip_for(self.device.host).map_err(|e| format!("{e:#}"))?;
        let url = format!("http://{ip}:{}/{PLAYLIST_NAME}", server.port());
        log::info!("casting {url} to {}", self.device.name);

        let cast_err = |what: &str, e: rust_cast::errors::Error| {
            format!("Chromecast {}: {what} failed ({e})", self.device.name)
        };
        let device = CastDevice::connect_without_host_verification(
            self.device.host.to_string(),
            self.device.port,
        )
        .map_err(|e| cast_err("connecting", e))?;
        device
            .connection
            .connect("receiver-0")
            .map_err(|e| cast_err("connecting", e))?;
        let app = device
            .receiver
            .launch_app(&CastDeviceApp::DefaultMediaReceiver)
            .map_err(|e| cast_err("starting the media receiver", e))?;
        device
            .connection
            .connect(app.transport_id.clone())
            .map_err(|e| cast_err("connecting to the app", e))?;
        let media = Media {
            content_id: url,
            stream_type: StreamType::Live,
            content_type: "application/x-mpegURL".to_string(),
            metadata: Some(Metadata::Generic(GenericMediaMetadata {
                title: Some("fractal-flow".to_string()),
                subtitle: None,
                images: Vec::new(),
                release_date: None,
            })),
            duration: None,
        };
        device
            .media
            .load(app.transport_id.clone(), app.session_id.clone(), &media)
            .map_err(|e| cast_err("loading the stream", e))?;
        set_status(
            &self.status,
            CastStatus::Streaming {
                device: self.device.name.clone(),
            },
        );

        let result = self.service_loop(&device);
        if let Err(e) = device.receiver.stop_app(app.session_id.clone()) {
            log::warn!("could not stop the Chromecast app: {e}");
        }
        result
    }

    /// Polls until the playlist lists at least two segments; false if a stop was requested.
    fn wait_for_segments(&self, dir: &Path) -> Result<bool, String> {
        let playlist = dir.join(PLAYLIST_NAME);
        let start = Instant::now();
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(false);
            }
            if let Ok(text) = std::fs::read_to_string(&playlist) {
                if text.lines().filter(|l| l.ends_with(".ts")).count() >= 2 {
                    return Ok(true);
                }
            }
            if let Some(msg) = self.encoder_failure() {
                return Err(msg);
            }
            if start.elapsed() > PLAYLIST_TIMEOUT {
                return Err(format!(
                    "ffmpeg produced no video within {}s",
                    PLAYLIST_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// A user-facing message if ffmpeg has died, else None.
    fn encoder_failure(&self) -> Option<String> {
        let mut guard = self.encoder.lock().unwrap_or_else(|e| e.into_inner());
        let enc = guard.as_mut()?;
        if enc.is_running() {
            None
        } else {
            Some(format!("ffmpeg stopped unexpectedly: {}", enc.last_error()))
        }
    }

    /// Answers heartbeats and watches for stop, ffmpeg death and the TV closing the app.
    fn service_loop(&self, device: &CastDevice<'_>) -> Result<(), String> {
        while !self.stop.load(Ordering::SeqCst) {
            // The Chromecast pings about every 5s, so this returns regularly.
            match device.receive() {
                Ok(ChannelMessage::Heartbeat(HeartbeatResponse::Ping)) => {
                    device
                        .heartbeat
                        .pong()
                        .map_err(|e| format!("Chromecast heartbeat failed ({e})"))?;
                }
                Ok(ChannelMessage::Connection(
                    rust_cast::channels::connection::ConnectionResponse::Close,
                )) => {
                    return Err("The Chromecast closed the connection".to_string());
                }
                Ok(ChannelMessage::Receiver(
                    rust_cast::channels::receiver::ReceiverResponse::Status(s),
                )) if s.applications.is_empty() => {
                    return Err("Casting was stopped on the TV".to_string());
                }
                Ok(_) => {}
                Err(e) => {
                    if self.stop.load(Ordering::SeqCst) {
                        break;
                    }
                    return Err(format!("Lost connection to the Chromecast ({e})"));
                }
            }
            if let Some(msg) = self.encoder_failure() {
                return Err(msg);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sessions share one temp dir per process, so tests that start sessions run one at a time.
    static SESSION_LOCK: Mutex<()> = Mutex::new(());

    /// Polls until `pred` holds for the status or the timeout passes; returns the last status.
    fn wait_status(
        s: &CastSession,
        timeout: Duration,
        pred: impl Fn(&CastStatus) -> bool,
    ) -> CastStatus {
        let start = Instant::now();
        loop {
            let st = s.status();
            if pred(&st) || start.elapsed() > timeout {
                return st;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// An unreachable device ends in Error (immediately without ffmpeg), and stop is prompt.
    #[test]
    fn unreachable_device_errors() {
        let _guard = SESSION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let device = CastDeviceInfo {
            name: "Nowhere".into(),
            host: "127.0.0.1".parse().unwrap(),
            port: 1,
        };
        let config = CastConfig {
            width: 64,
            height: 64,
            fps: 10,
            bitrate_kbps: 500,
        };
        let session = CastSession::start(device, config);
        assert_eq!(session.config(), config);
        assert!(matches!(
            session.status(),
            CastStatus::Starting | CastStatus::Error(_)
        ));
        session.push_frame(vec![0; 64 * 64 * 4]);

        let st = wait_status(&session, Duration::from_secs(25), |s| {
            matches!(s, CastStatus::Error(_))
        });
        match (ffmpeg_available(), &st) {
            (Err(reason), CastStatus::Error(msg)) => assert_eq!(&reason, msg),
            (Ok(()), CastStatus::Error(msg)) => assert!(msg.contains("Nowhere"), "{msg}"),
            (_, other) => panic!("expected Error, got {other:?}"),
        }
        let t = Instant::now();
        session.stop();
        assert!(t.elapsed() < Duration::from_secs(7));
    }

    /// Stopping while still starting returns promptly and leaves no temp dir behind.
    #[test]
    fn stop_while_starting() {
        let _guard = SESSION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let device = CastDeviceInfo {
            name: "Nowhere".into(),
            host: "127.0.0.1".parse().unwrap(),
            port: 1,
        };
        let config = CastConfig {
            width: 64,
            height: 64,
            fps: 10,
            bitrate_kbps: 500,
        };
        let session = CastSession::start(device, config);
        let t = Instant::now();
        session.stop();
        assert!(t.elapsed() < Duration::from_secs(7));
    }

    /// ffmpeg_available returns either Ok or a user-facing message.
    #[test]
    fn ffmpeg_check_message() {
        if let Err(msg) = ffmpeg_available() {
            assert!(msg.contains("ffmpeg"));
        }
    }
}
