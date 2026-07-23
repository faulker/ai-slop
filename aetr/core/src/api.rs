//! UniFFI surface: the `AetrSession` object the platform apps talk to.
//!
//! One session = one passphrase + modem mode. `encode_text`/`encode_voice`
//! return complete 48 kHz mono f32 bursts ready to key up; `push_rx` is fed
//! raw microphone PCM in any block size and the UI drains `poll_events` at
//! ~10 Hz. The ARQ round is receiver-initiated (`request_repair`) and the
//! sender side answers automatically inside `push_rx` by queueing a
//! `RxEvent::RepairRequested` carrying the ready-to-transmit repair PCM.
//! All state sits behind a Mutex; nothing here panics across the FFI.

use crate::frame::{self, Reassembler, TxCache};
use crate::modem::{burst_samples, Modem, ModemMode, OfdmModem, OfdmRx, RxStatus};
use crate::{crypto, fec, voice, AetrError};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Silence between consecutive bursts of one transmission (0.1 s), giving
/// the receiver's block decoder room to finalize each frame.
const GAP_SAMPLES: usize = 4800;
/// Lead-in silence before the first burst (radio keying/VOX settle).
const LEAD_SAMPLES: usize = 4800;
/// Trailing silence so the receiver flushes its last decoder blocks before
/// the sender unkeys (0.5 s, also rides out squelch tails).
const TAIL_SAMPLES: usize = 24000;
/// Reassembly timeout: an in-flight message older than this finalizes
/// (voice with silence gaps) or fails (text) on the next `poll_events`.
const RX_TIMEOUT: Duration = Duration::from_secs(30);
/// Sender-side ARQ caches kept per session before evicting the oldest.
const TX_CACHE_LIMIT: usize = 32;
/// Default voice clip cap in seconds (user-configurable in SessionConfig).
pub const DEFAULT_VOICE_CAP_SECS: u32 = 30;

/// Receiver state for UI badges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RxState {
    /// No signal being tracked.
    Idle,
    /// A burst preamble was detected; payload symbols are being collected.
    Syncing,
    /// At least one message is partially reassembled.
    Receiving,
}

/// What a payload is, for airtime estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PayloadKind {
    /// `len` is the UTF-8 byte count of the text.
    Text,
    /// `len` is the clip length in 48 kHz samples.
    Voice,
}

/// Session parameters. The passphrase is the only shared secret; the mode
/// must match on both ends of the channel.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SessionConfig {
    /// Shared passphrase; fed through Argon2id to derive the session key.
    pub passphrase: String,
    /// Modem payload mode (85/128/170 bytes per burst).
    pub mode: ModemMode,
    /// Maximum voice clip length accepted by `encode_voice`, in seconds.
    #[uniffi(default = 30)]
    pub voice_cap_secs: u32,
    /// Extra wait before the first burst, in milliseconds, giving the radio
    /// time to finish keying up (Bluetooth/VOX radios can take ~1 s). Added
    /// on top of the fixed 0.1 s lead-in.
    #[uniffi(default = 1000)]
    pub tx_delay_ms: u32,
    /// Fill the TX delay with a quiet primer tone instead of silence, so
    /// radios that key on audio energy (VOX) open the transmitter during
    /// the delay rather than clipping the start of the data.
    #[uniffi(default = false)]
    pub vox_primer: bool,
}

/// Per-session transmit timing derived from `SessionConfig`.
#[derive(Debug, Clone, Copy)]
struct TxTiming {
    /// Extra key-up delay before the first burst, in samples.
    delay_samples: usize,
    /// Whether the delay carries a primer tone instead of silence.
    primer: bool,
}

/// Events drained by the UI via `poll_events`.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum RxEvent {
    /// A complete text message.
    Text {
        /// Message id it arrived under.
        message_id: u64,
        /// Decoded UTF-8 text.
        text: String,
    },
    /// A voice clip, possibly with silent gaps where spans were lost.
    Voice {
        /// Message id it arrived under.
        message_id: u64,
        /// 48 kHz mono PCM ready to play.
        pcm48k: Vec<f32>,
        /// Span indices that were missing and filled with silence.
        missing_spans: Vec<u32>,
    },
    /// Reassembly progress for an in-flight message (emitted on change).
    Progress {
        /// Message being assembled.
        message_id: u64,
        /// Distinct shards held so far.
        received: u32,
        /// Shards needed to complete (K for text, span count for voice).
        total: u32,
        /// Whether the message is voice.
        is_voice: bool,
    },
    /// A receiver asked us to repair a message we sent. `pcm_response` is
    /// the ready-to-transmit repair burst; the app decides when to key up.
    RepairRequested {
        /// Our message the peer wants repaired.
        message_id: u64,
        /// Complete 48 kHz PCM answering the request.
        pcm_response: Vec<f32>,
    },
    /// A message that timed out or an internal receive-path error
    /// (message_id 0 when no message is attributable).
    Failed {
        /// Message id of the failed message, or 0.
        message_id: u64,
        /// Why it failed.
        reason: String,
    },
}

/// Mutable session state guarded by the session Mutex.
struct Inner {
    /// Streaming COFDM receiver.
    rx: OfdmRx,
    /// Decrypt + reassemble state.
    reassembler: Reassembler,
    /// Events queued for the next `poll_events`.
    events: Vec<RxEvent>,
    /// Per-message ARQ caches for messages we transmitted.
    tx_caches: HashMap<u64, TxCache>,
    /// Insertion order of `tx_caches` for oldest-first eviction.
    tx_order: VecDeque<u64>,
    /// Last emitted progress (shards held) per in-flight message.
    progress_seen: HashMap<u64, usize>,
}

impl Inner {
    /// Stores a transmitted message's ARQ cache, evicting the oldest cache
    /// once the session holds more than TX_CACHE_LIMIT.
    fn cache_tx(&mut self, cache: TxCache) {
        let id = cache.message_id;
        if self.tx_caches.insert(id, cache).is_none() {
            self.tx_order.push_back(id);
            if self.tx_order.len() > TX_CACHE_LIMIT {
                if let Some(old) = self.tx_order.pop_front() {
                    self.tx_caches.remove(&old);
                }
            }
        }
    }

    /// Maps a core reassembly event into the FFI event queue. Control
    /// frames targeting one of our cached messages are answered here by
    /// queueing a ready-to-transmit `RepairRequested` burst.
    fn handle_frame_event(
        &mut self,
        ev: frame::RxEvent,
        key: &[u8; 32],
        mode: ModemMode,
        timing: TxTiming,
    ) {
        match ev {
            frame::RxEvent::Text { message_id, text } => {
                self.events.push(RxEvent::Text { message_id, text })
            }
            frame::RxEvent::Voice { message_id, pcm48k, missing_spans } => {
                self.events.push(RxEvent::Voice { message_id, pcm48k, missing_spans })
            }
            frame::RxEvent::Control { request } => {
                let target = request.target;
                // Not a message we sent: someone else's repair round.
                let Some(cache) = self.tx_caches.get(&target) else { return };
                let repair = frame::build_repair_frames(key, cache, &request).and_then(|frames| {
                    if frames.is_empty() {
                        Ok(None) // Peer already holds enough shards.
                    } else {
                        modulate_frames(mode, &frames, timing).map(Some)
                    }
                });
                match repair {
                    Ok(Some(pcm)) => self
                        .events
                        .push(RxEvent::RepairRequested { message_id: target, pcm_response: pcm }),
                    Ok(None) => {}
                    Err(e) => self
                        .events
                        .push(RxEvent::Failed { message_id: target, reason: e.to_string() }),
                }
            }
            frame::RxEvent::Failed { message_id, reason } => {
                self.events.push(RxEvent::Failed { message_id, reason })
            }
        }
    }

    /// Emits a Progress event for every in-flight message whose shard count
    /// changed since the last call, and drops tracking for finished ones.
    fn emit_progress(&mut self) {
        let snapshot = self.reassembler.progress();
        let mut seen = HashMap::with_capacity(snapshot.len());
        for (id, held, needed, is_voice) in snapshot {
            if self.progress_seen.get(&id) != Some(&held) {
                self.events.push(RxEvent::Progress {
                    message_id: id,
                    received: held as u32,
                    total: needed as u32,
                    is_voice,
                });
            }
            seen.insert(id, held);
        }
        self.progress_seen = seen;
    }
}

/// Frequency of the optional VOX primer tone (in the FM voice passband).
const PRIMER_HZ: f32 = 700.0;
/// Primer tone amplitude: loud enough to trip VOX, well below data level.
const PRIMER_AMPLITUDE: f32 = 0.3;
/// Raised-cosine fade at each end of the primer tone (5 ms) to avoid clicks.
const PRIMER_FADE_SAMPLES: usize = 240;

/// Builds the key-up lead for one transmission: `timing.delay_samples` of
/// silence (or a faded primer tone when `timing.primer` is set) followed by
/// the fixed 0.1 s silent lead-in that separates keying from data.
fn build_lead(timing: TxTiming) -> Vec<f32> {
    let mut lead = vec![0.0f32; timing.delay_samples + LEAD_SAMPLES];
    if timing.primer && timing.delay_samples > 0 {
        let n = timing.delay_samples;
        let fade = PRIMER_FADE_SAMPLES.min(n / 2);
        for i in 0..n {
            let mut a = PRIMER_AMPLITUDE;
            if i < fade {
                a *= 0.5 - 0.5 * (std::f32::consts::PI * i as f32 / fade as f32).cos();
            } else if i >= n - fade {
                let j = n - 1 - i;
                a *= 0.5 - 0.5 * (std::f32::consts::PI * j as f32 / fade as f32).cos();
            }
            lead[i] = a * (2.0 * std::f32::consts::PI * PRIMER_HZ * i as f32 / 48_000.0).sin();
        }
    }
    lead
}

/// Concatenates the bursts for a frame list into one transmission:
/// key-up lead (configurable delay + fixed lead-in), each burst followed by
/// an inter-burst gap, then a tail so the receiver flushes before the
/// sender unkeys.
fn modulate_frames(
    mode: ModemMode,
    frames: &[Vec<u8>],
    timing: TxTiming,
) -> Result<Vec<f32>, AetrError> {
    let modem = OfdmModem;
    let mut pcm = build_lead(timing);
    for f in frames {
        pcm.extend(modem.encode_frame(mode, f)?);
        pcm.extend(std::iter::repeat(0.0f32).take(GAP_SAMPLES));
    }
    pcm.extend(std::iter::repeat(0.0f32).take(TAIL_SAMPLES));
    Ok(pcm)
}

/// Estimates on-air seconds for a payload in a given mode, including the
/// TX key-up delay, lead-in, inter-burst gaps, and tail. For `Text`, `len`
/// is UTF-8 bytes; for `Voice`, `len` is 48 kHz samples. The UIs use this
/// to show the cost of longer voice caps, robust modes, and TX delay
/// before transmitting.
#[uniffi::export]
pub fn estimate_airtime_secs(mode: ModemMode, kind: PayloadKind, len: u64, tx_delay_ms: u32) -> f64 {
    let payload = mode.chunk_payload_bytes();
    let chunks = match kind {
        PayloadKind::Text => {
            // Mirrors build_text_message: u16 length prefix + bytes.
            let body = 2 + len as usize;
            let k = body.div_ceil(payload).max(1);
            fec::transmit_count(k)
        }
        PayloadKind::Voice => {
            // Mirrors encode_clip: 6x decimation, 40 ms codec2 frames.
            let frames_8k = (len as usize / voice::RESAMPLE_FACTOR)
                .div_ceil(voice::CODEC2_FRAME_SAMPLES_8K)
                .max(1);
            frames_8k.div_ceil(voice::frames_per_chunk(payload).max(1))
        }
    };
    let delay_samples = tx_delay_ms as usize * 48;
    let samples = delay_samples + LEAD_SAMPLES + chunks * (burst_samples() + GAP_SAMPLES) + TAIL_SAMPLES;
    samples as f64 / 48_000.0
}

/// One encrypted text/voice session over one audio channel. Construction
/// runs the Argon2id KDF (blocking, ~100 ms); everything after is cheap.
#[derive(uniffi::Object)]
pub struct AetrSession {
    /// Derived 32-byte session key.
    key: [u8; 32],
    /// Modem payload mode for everything this session sends and estimates.
    mode: ModemMode,
    /// Maximum accepted voice clip length in seconds.
    voice_cap_secs: u32,
    /// TX key-up delay and primer-tone setting for everything this session
    /// transmits (messages, repair requests, repair responses).
    timing: TxTiming,
    /// All mutable state (receiver, reassembler, queues, ARQ caches).
    inner: Mutex<Inner>,
}

impl AetrSession {
    /// Locks the session state, recovering from a poisoned Mutex instead of
    /// panicking across the FFI.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[uniffi::export]
impl AetrSession {
    /// Creates a session: derives the key from the passphrase (blocking
    /// Argon2id, ~100 ms — call off the UI thread) and allocates the
    /// streaming receiver.
    #[uniffi::constructor]
    pub fn new(config: SessionConfig) -> Result<Arc<Self>, AetrError> {
        let key = crypto::derive_key(&config.passphrase)?;
        let rx = OfdmRx::new()?;
        Ok(Arc::new(AetrSession {
            key,
            mode: config.mode,
            voice_cap_secs: config.voice_cap_secs,
            timing: TxTiming {
                delay_samples: config.tx_delay_ms as usize * 48,
                primer: config.vox_primer,
            },
            inner: Mutex::new(Inner {
                rx,
                reassembler: Reassembler::new(key),
                events: Vec::new(),
                tx_caches: HashMap::new(),
                tx_order: VecDeque::new(),
                progress_seen: HashMap::new(),
            }),
        }))
    }

    /// Encodes a text message into one complete 48 kHz PCM transmission
    /// (all bursts, gaps, lead-in/tail) and caches the RS parity pool for a
    /// later ARQ round.
    pub fn encode_text(&self, text: String) -> Result<Vec<f32>, AetrError> {
        let tx = frame::build_text_message(&self.key, self.mode, &text, frame::random_message_id())?;
        let pcm = modulate_frames(self.mode, &tx.frames, self.timing)?;
        self.lock().cache_tx(tx.cache);
        Ok(pcm)
    }

    /// Encodes a 48 kHz mono voice clip into one complete PCM transmission.
    /// Rejects clips longer than the configured cap; caches the spans for a
    /// later exact-retransmission ARQ round.
    pub fn encode_voice(&self, pcm48k: Vec<f32>) -> Result<Vec<f32>, AetrError> {
        let cap = self.voice_cap_secs as usize * 48_000;
        if pcm48k.len() > cap {
            return Err(AetrError::TooLarge(format!(
                "clip is {:.1} s, cap is {} s",
                pcm48k.len() as f64 / 48_000.0,
                self.voice_cap_secs
            )));
        }
        let tx = frame::build_voice_message(&self.key, self.mode, &pcm48k, frame::random_message_id())?;
        let pcm = modulate_frames(self.mode, &tx.frames, self.timing)?;
        self.lock().cache_tx(tx.cache);
        Ok(pcm)
    }

    /// Feeds received PCM (any block size, called from the audio thread).
    /// Cheap: buffers into the modem, decrypts any completed frames, and
    /// queues events — including automatic `RepairRequested` responses when
    /// a peer's control frame targets one of our cached messages. Frames
    /// that fail authentication (wrong passphrase on the channel) or are
    /// malformed are silently dropped, as on any shared radio channel.
    pub fn push_rx(&self, pcm48k: Vec<f32>) {
        let mut inner = self.lock();
        let frames = match inner.rx.feed(&pcm48k) {
            Ok(frames) => frames,
            Err(e) => {
                inner.events.push(RxEvent::Failed { message_id: 0, reason: e.to_string() });
                return;
            }
        };
        let now = Instant::now();
        for f in frames {
            match inner.reassembler.push_frame(&f, now) {
                Ok(Some(ev)) => inner.handle_frame_event(ev, &self.key, self.mode, self.timing),
                Ok(None) => {}
                Err(_) => {} // AuthFailed/Malformed: not for us, drop.
            }
        }
        inner.emit_progress();
    }

    /// Drains queued events. Also sweeps the reassembly timeout: stale
    /// voice finalizes with silence gaps, stale text becomes `Failed`.
    /// Poll at ~10 Hz from the UI.
    pub fn poll_events(&self) -> Vec<RxEvent> {
        let mut inner = self.lock();
        let expired = inner.reassembler.take_expired(Instant::now(), RX_TIMEOUT);
        for ev in expired {
            inner.handle_frame_event(ev, &self.key, self.mode, self.timing);
        }
        std::mem::take(&mut inner.events)
    }

    /// Current receiver state for a UI badge.
    pub fn rx_state(&self) -> RxState {
        let inner = self.lock();
        if inner.reassembler.pending() > 0 {
            RxState::Receiving
        } else if inner.rx.status() == RxStatus::Synced {
            RxState::Syncing
        } else {
            RxState::Idle
        }
    }

    /// Discards all receive state (in-flight messages, queued events,
    /// modem sync). Keeps the key, mode, and sender-side ARQ caches.
    pub fn reset_rx(&self) {
        let mut inner = self.lock();
        if let Ok(rx) = OfdmRx::new() {
            inner.rx = rx;
        }
        inner.reassembler = Reassembler::new(self.key);
        inner.events.clear();
        inner.progress_seen.clear();
    }

    /// Builds the receiver side of the ARQ round: a single control-frame
    /// transmission asking the original sender to repair `message_id`.
    /// Errors if the message is unknown or already complete. The sender's
    /// session answers automatically via `RxEvent::RepairRequested`.
    pub fn request_repair(&self, message_id: u64) -> Result<Vec<f32>, AetrError> {
        let request = self
            .lock()
            .reassembler
            .build_repair_request(message_id)
            .ok_or_else(|| {
                AetrError::Malformed("message unknown or already complete".into())
            })?;
        let control =
            frame::build_control_frame(&self.key, self.mode, &request, frame::random_message_id())?;
        modulate_frames(self.mode, &[control], self.timing)
    }

    /// The configured voice clip cap in seconds.
    pub fn voice_cap_secs(&self) -> u32 {
        self.voice_cap_secs
    }

    /// The configured TX key-up delay in milliseconds.
    pub fn tx_delay_ms(&self) -> u32 {
        (self.timing.delay_samples / 48) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Config both ends of a loopback share in these tests.
    fn config() -> SessionConfig {
        SessionConfig {
            passphrase: "correct horse battery staple".into(),
            mode: ModemMode::B170,
            voice_cap_secs: DEFAULT_VOICE_CAP_SECS,
            tx_delay_ms: 0,
            vox_primer: false,
        }
    }

    /// Builds a text of exactly `len` ASCII bytes with distinctive content.
    fn long_text(len: usize) -> String {
        let base = "the quick brown fox jumps over the lazy dog 0123456789 ";
        base.chars().cycle().take(len).collect()
    }

    #[test]
    fn session_text_roundtrip() {
        let sender = AetrSession::new(config()).expect("sender");
        let receiver = AetrSession::new(config()).expect("receiver");

        let pcm = sender.encode_text("hello over fm".into()).expect("encode");
        assert!(pcm.len() > burst_samples());
        receiver.push_rx(pcm);
        let events = receiver.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                RxEvent::Text { text, .. } if text == "hello over fm"
            )),
            "expected the text event, got {events:?}"
        );
        assert_eq!(receiver.rx_state(), RxState::Idle);
    }

    #[test]
    fn session_voice_roundtrip() {
        let sender = AetrSession::new(config()).expect("sender");
        let receiver = AetrSession::new(config()).expect("receiver");

        // 0.5 s of 440 Hz tone at 48 kHz.
        let clip: Vec<f32> = (0..24_000)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        let pcm = sender.encode_voice(clip).expect("encode");
        receiver.push_rx(pcm);
        let events = receiver.poll_events();
        let voice = events.iter().find_map(|e| match e {
            RxEvent::Voice { pcm48k, missing_spans, .. } => Some((pcm48k, missing_spans)),
            _ => None,
        });
        let (pcm48k, missing) = voice.expect("voice event");
        assert!(missing.is_empty());
        let energy: f32 = pcm48k.iter().map(|s| s * s).sum();
        assert!(energy > 1.0, "decoded clip should carry energy, got {energy}");
    }

    #[test]
    fn session_arq_repair_flow() {
        let sender = AetrSession::new(config()).expect("sender");
        let receiver = AetrSession::new(config()).expect("receiver");

        // 1400 bytes -> K = 10 source + 3 repair = 13 bursts.
        let text = long_text(1400);
        let pcm = sender.encode_text(text.clone()).expect("encode");
        let stride = burst_samples() + GAP_SAMPLES;
        assert_eq!(pcm.len(), LEAD_SAMPLES + 13 * stride + TAIL_SAMPLES);

        // Drop the first 4 bursts: 9 distinct shards < K = 10, incomplete.
        let mut damaged = pcm[..LEAD_SAMPLES].to_vec();
        damaged.extend_from_slice(&pcm[LEAD_SAMPLES + 4 * stride..]);
        receiver.push_rx(damaged);
        let events = receiver.poll_events();
        assert!(
            !events.iter().any(|e| matches!(e, RxEvent::Text { .. })),
            "9 of 10 shards must not complete: {events:?}"
        );
        let (id, received, total) = events
            .iter()
            .find_map(|e| match e {
                RxEvent::Progress { message_id, received, total, is_voice: false } => {
                    Some((*message_id, *received, *total))
                }
                _ => None,
            })
            .expect("progress event");
        assert_eq!((received, total), (9, 10));
        assert_eq!(receiver.rx_state(), RxState::Receiving);

        // Receiver asks for repair; the sender's push_rx answers by itself.
        let control = receiver.request_repair(id).expect("repair request");
        sender.push_rx(control);
        let sender_events = sender.poll_events();
        let response = sender_events
            .iter()
            .find_map(|e| match e {
                RxEvent::RepairRequested { message_id, pcm_response } if *message_id == id => {
                    Some(pcm_response.clone())
                }
                _ => None,
            })
            .expect("repair response");
        // One parity shard closes a 1-shard gap.
        assert_eq!(response.len(), LEAD_SAMPLES + stride + TAIL_SAMPLES);

        receiver.push_rx(response);
        let events = receiver.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                RxEvent::Text { message_id, text: t } if *message_id == id && *t == text
            )),
            "repair round should complete the text: {events:?}"
        );
    }

    #[test]
    fn voice_cap_is_enforced() {
        let mut cfg = config();
        cfg.voice_cap_secs = 1;
        let session = AetrSession::new(cfg).expect("session");
        let long_clip = vec![0.1f32; 48_000 + 1];
        match session.encode_voice(long_clip) {
            Err(AetrError::TooLarge(_)) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
        assert!(session.encode_voice(vec![0.1f32; 24_000]).is_ok());
        assert_eq!(session.voice_cap_secs(), 1);
    }

    #[test]
    fn airtime_estimates_match_encoded_output() {
        // Single-chunk text: exactly one burst plus framing silence.
        let expected =
            (LEAD_SAMPLES + burst_samples() + GAP_SAMPLES + TAIL_SAMPLES) as f64 / 48_000.0;
        let est = estimate_airtime_secs(ModemMode::B170, PayloadKind::Text, 10, 0);
        assert!((est - expected).abs() < 1e-9, "single chunk estimate {est} != {expected}");

        // Estimates grow with payload and with more robust (smaller) modes.
        let small = estimate_airtime_secs(ModemMode::B170, PayloadKind::Text, 1400, 0);
        let robust = estimate_airtime_secs(ModemMode::B85, PayloadKind::Text, 1400, 0);
        assert!(robust > small);

        // 30 s voice cap: the estimate the settings UI shows.
        let voice = estimate_airtime_secs(ModemMode::B170, PayloadKind::Voice, 30 * 48_000, 0);
        assert!(voice > 30.0, "voice airtime exceeds clip length, got {voice}");

        // The TX key-up delay is included in the estimate.
        let delayed = estimate_airtime_secs(ModemMode::B170, PayloadKind::Text, 10, 1000);
        assert!((delayed - expected - 1.0).abs() < 1e-9, "1 s delay adds 1 s, got {delayed}");
    }

    #[test]
    fn tx_delay_and_primer_prepend_keyup_lead() {
        let mut cfg = config();
        cfg.tx_delay_ms = 1000;
        cfg.vox_primer = true;
        let sender = AetrSession::new(cfg).expect("sender");
        let receiver = AetrSession::new(config()).expect("receiver");

        let pcm = sender.encode_text("keyed up".into()).expect("encode");
        assert_eq!(sender.tx_delay_ms(), 1000);
        let delay = 48_000; // 1000 ms at 48 kHz
        assert_eq!(
            pcm.len(),
            delay + LEAD_SAMPLES + burst_samples() + GAP_SAMPLES + TAIL_SAMPLES
        );

        // Primer tone carries energy through the delay, then the fixed
        // lead-in stays silent so keying noise never touches the data.
        let primer_energy: f32 = pcm[..delay].iter().map(|s| s * s).sum();
        assert!(primer_energy > 100.0, "primer should carry tone energy, got {primer_energy}");
        let lead_energy: f32 = pcm[delay..delay + LEAD_SAMPLES].iter().map(|s| s * s).sum();
        assert_eq!(lead_energy, 0.0, "fixed lead-in must stay silent");

        // A receiver with a different (zero) TX delay still decodes it.
        receiver.push_rx(pcm);
        let events = receiver.poll_events();
        assert!(
            events.iter().any(|e| matches!(
                e,
                RxEvent::Text { text, .. } if text == "keyed up"
            )),
            "delayed+primed transmission should decode: {events:?}"
        );
    }
}
