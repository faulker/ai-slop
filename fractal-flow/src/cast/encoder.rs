//! ffmpeg child process that turns raw RGBA frames into a live HLS stream.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Context;

use super::CastConfig;

/// File name of the HLS playlist inside the output directory.
pub const PLAYLIST_NAME: &str = "stream.m3u8";

/// How many trailing ffmpeg stderr lines are kept for error messages.
const STDERR_LINES: usize = 20;

type FrameSlot = Arc<Mutex<Option<Arc<Vec<u8>>>>>;

/// Builds the ffmpeg argv (without the program name) for a raw RGBA stdin to HLS pipeline.
pub fn ffmpeg_args(config: &CastConfig, out_dir: &Path) -> Vec<String> {
    let bitrate = config.bitrate_kbps;
    let dir = out_dir.display();
    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostats".into(),
        "-loglevel".into(),
        "warning".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "-s".into(),
        format!("{}x{}", config.width, config.height),
        "-r".into(),
        config.fps.to_string(),
        "-i".into(),
        "-".into(),
        "-c:v".into(),
        "h264_videotoolbox".into(),
        "-realtime".into(),
        "1".into(),
        "-b:v".into(),
        format!("{bitrate}k"),
        "-maxrate".into(),
        format!("{bitrate}k"),
        "-bufsize".into(),
        format!("{}k", bitrate * 2),
        "-g".into(),
        config.fps.to_string(),
        "-keyint_min".into(),
        config.fps.to_string(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-profile:v".into(),
        "high".into(),
        "-an".into(),
        "-f".into(),
        "hls".into(),
        "-hls_time".into(),
        "1".into(),
        "-hls_list_size".into(),
        "6".into(),
        "-hls_flags".into(),
        "delete_segments+independent_segments+omit_endlist".into(),
        "-hls_segment_filename".into(),
        format!("{dir}/seg%05d.ts"),
        format!("{dir}/{PLAYLIST_NAME}"),
    ];
    args
}

/// Runs ffmpeg and feeds it frames at a fixed rate from a shared latest-frame slot.
pub struct Encoder {
    child: Child,
    slot: FrameSlot,
    frame_len: usize,
    stop: Arc<AtomicBool>,
    writer: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

impl Encoder {
    /// Spawns ffmpeg writing HLS into `out_dir` and starts the fixed-rate writer thread.
    pub fn start(config: CastConfig, out_dir: &Path) -> anyhow::Result<Encoder> {
        anyhow::ensure!(
            config.width > 0 && config.height > 0 && config.fps > 0,
            "invalid cast configuration"
        );
        let mut child = Command::new("ffmpeg")
            .args(ffmpeg_args(&config, out_dir))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("ffmpeg not found. Install it with: brew install ffmpeg")?;
        let stdin = child.stdin.take().context("ffmpeg stdin unavailable")?;
        let stderr = child.stderr.take().context("ffmpeg stderr unavailable")?;

        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        let tail = stderr_tail.clone();
        let stderr_thread = std::thread::Builder::new()
            .name("cast-ffmpeg-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    log::debug!("ffmpeg: {line}");
                    let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                    if tail.len() == STDERR_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            })
            .context("could not spawn the ffmpeg stderr thread")?;

        let frame_len = config.width as usize * config.height as usize * 4;
        let slot: FrameSlot = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let slot = slot.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("cast-frame-writer".into())
                .spawn(move || write_loop(stdin, &slot, &stop, frame_len, config.fps))
                .context("could not spawn the frame writer thread")?
        };

        Ok(Encoder {
            child,
            slot,
            frame_len,
            stop,
            writer: Some(writer),
            stderr_thread: Some(stderr_thread),
            stderr_tail,
        })
    }

    /// Replaces the latest-frame slot; frames of the wrong size are dropped with a warning.
    pub fn push_frame(&self, rgba: Vec<u8>) {
        if rgba.len() != self.frame_len {
            log::warn!(
                "dropping cast frame of {} bytes, expected {}",
                rgba.len(),
                self.frame_len
            );
            return;
        }
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(rgba));
    }

    /// False once ffmpeg has exited (or its status cannot be read).
    pub fn is_running(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                log::error!("ffmpeg exited: {status}");
                false
            }
            Err(e) => {
                log::error!("could not query ffmpeg: {e}");
                false
            }
        }
    }

    /// The last lines ffmpeg printed to stderr, joined for display.
    pub fn last_error(&self) -> String {
        let tail = self.stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
        tail.iter()
            .rev()
            .take(3)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Stops the writer, closes ffmpeg's input and kills the process.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(w) = self.writer.take() {
            if w.join().is_err() {
                log::error!("frame writer thread panicked");
            }
        }
        if let Err(e) = self.child.kill() {
            log::debug!("ffmpeg kill: {e}");
        }
        if let Err(e) = self.child.wait() {
            log::warn!("ffmpeg wait failed: {e}");
        }
        if let Some(t) = self.stderr_thread.take() {
            if t.join().is_err() {
                log::error!("ffmpeg stderr thread panicked");
            }
        }
    }
}

/// Writes one frame per tick (latest frame, or black before the first) until stopped or ffmpeg's pipe closes.
fn write_loop(
    mut stdin: ChildStdin,
    slot: &FrameSlot,
    stop: &AtomicBool,
    frame_len: usize,
    fps: u32,
) {
    let period = Duration::from_secs_f64(1.0 / fps as f64);
    let black: Arc<Vec<u8>> = Arc::new([0u8, 0, 0, 255].repeat(frame_len / 4));
    let mut next = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        let frame = slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| black.clone());
        if let Err(e) = stdin.write_all(&frame) {
            if !stop.load(Ordering::SeqCst) {
                log::error!("writing to ffmpeg failed: {e}");
            }
            return;
        }
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > period * 4 {
            // Fell far behind (e.g. ffmpeg stalled); resync instead of bursting.
            next = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Returns the value following `flag` in the argv.
    fn value_of<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("missing {flag}"));
        &args[i + 1]
    }

    /// The argv carries size, rate, bitrate, GOP and output paths.
    #[test]
    fn args_contain_config() {
        let cfg = CastConfig {
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_kbps: 6000,
        };
        let dir = PathBuf::from("/tmp/x");
        let args = ffmpeg_args(&cfg, &dir);
        assert_eq!(value_of(&args, "-s"), "1280x720");
        assert_eq!(value_of(&args, "-r"), "30");
        assert_eq!(value_of(&args, "-b:v"), "6000k");
        assert_eq!(value_of(&args, "-maxrate"), "6000k");
        assert_eq!(value_of(&args, "-bufsize"), "12000k");
        assert_eq!(value_of(&args, "-g"), "30");
        assert_eq!(value_of(&args, "-keyint_min"), "30");
        assert_eq!(value_of(&args, "-c:v"), "h264_videotoolbox");
        assert_eq!(
            value_of(&args, "-hls_segment_filename"),
            "/tmp/x/seg%05d.ts"
        );
        assert_eq!(args.last().unwrap(), "/tmp/x/stream.m3u8");
        assert_eq!(value_of(&args, "-i"), "-");
    }

    /// Real ffmpeg produces a playlist and at least one segment.
    #[test]
    fn ffmpeg_end_to_end() {
        if let Err(reason) = super::super::ffmpeg_available() {
            println!("skipping: {reason}");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let cfg = CastConfig {
            width: 64,
            height: 64,
            fps: 10,
            bitrate_kbps: 500,
        };
        let mut enc = Encoder::start(cfg, dir.path()).unwrap();
        let start = Instant::now();
        let mut shade = 0u8;
        while start.elapsed() < Duration::from_millis(2500) {
            shade = shade.wrapping_add(20);
            enc.push_frame([shade, 0, 255 - shade, 255].repeat(64 * 64));
            std::thread::sleep(Duration::from_millis(100));
        }
        let running = enc.is_running();
        let err = enc.last_error();
        enc.stop();
        assert!(running, "ffmpeg exited early: {err}");
        assert!(dir.path().join(PLAYLIST_NAME).exists());
        let segs = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".ts"))
            .count();
        assert!(segs >= 1, "no segments written");
    }

    /// Frames of the wrong size are ignored.
    #[test]
    fn wrong_size_frame_dropped() {
        if super::super::ffmpeg_available().is_err() {
            println!("skipping: ffmpeg unavailable");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let cfg = CastConfig {
            width: 16,
            height: 16,
            fps: 10,
            bitrate_kbps: 200,
        };
        let enc = Encoder::start(cfg, dir.path()).unwrap();
        enc.push_frame(vec![0; 10]);
        assert!(enc.slot.lock().unwrap().is_none());
        enc.stop();
    }
}
