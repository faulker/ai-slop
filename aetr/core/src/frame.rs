//! Chunk header, message chunking, reassembly, and the ARQ control frames.
//!
//! One chunk = one modem transmission. Frame layout on the wire:
//! `header (11 bytes, AEAD associated data) || ciphertext || tag (16 bytes)`,
//! always padded so the frame exactly fills the modem mode's payload
//! (85/128/170 bytes). Header: `message_id(8 LE) | chunk_index | chunk_count
//! | flags`. For multi-chunk text, chunk_count is the total transmitted
//! upfront (K source + R repair) and K is recovered via
//! [`crate::fec::source_count_from_total`]; for voice it is the span count.

use crate::modem::ModemMode;
use crate::{crypto, fec, voice, AetrError};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

/// Chunk header length in bytes.
pub const HEADER_LEN: usize = 11;
/// Fixed per-frame overhead: header plus Poly1305 tag.
pub const FRAME_OVERHEAD: usize = HEADER_LEN + crypto::TAG_LEN;
/// Highest chunk index the one-byte field may carry (255 is reserved).
pub const MAX_CHUNK_INDEX: usize = 254;

/// Header flag bit 0: this chunk is a Reed-Solomon repair shard.
pub const FLAG_REPAIR: u8 = 1 << 0;
/// Header flag bit 1: payload type (0 = text, 1 = voice).
pub const FLAG_VOICE: u8 = 1 << 1;
/// Header flag bit 2: final chunk of the upfront transmission.
pub const FLAG_LAST: u8 = 1 << 2;
/// Header flag bit 3: ARQ control frame (payload = target id + bitmask).
pub const FLAG_CONTROL: u8 = 1 << 3;

/// The 11-byte chunk header. Authenticated (AAD) but sent in the clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Random per-message identifier shared by all its chunks.
    pub message_id: u64,
    /// Position of this chunk: source 0..K-1, repair K..2K-1 for text.
    pub chunk_index: u8,
    /// Total chunks transmitted upfront (incl. repair); voice span count.
    pub chunk_count: u8,
    /// FLAG_* bits.
    pub flags: u8,
}

impl Header {
    /// Serializes the header to its 11-byte wire form (message_id LE).
    pub fn pack(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..8].copy_from_slice(&self.message_id.to_le_bytes());
        out[8] = self.chunk_index;
        out[9] = self.chunk_count;
        out[10] = self.flags;
        out
    }

    /// Parses an 11-byte wire header.
    pub fn unpack(bytes: &[u8]) -> Result<Header, AetrError> {
        if bytes.len() < HEADER_LEN {
            return Err(AetrError::Malformed("header shorter than 11 bytes".into()));
        }
        let mut id = [0u8; 8];
        id.copy_from_slice(&bytes[..8]);
        Ok(Header {
            message_id: u64::from_le_bytes(id),
            chunk_index: bytes[8],
            chunk_count: bytes[9],
            flags: bytes[10],
        })
    }

    /// True if this is an ARQ control frame.
    pub fn is_control(&self) -> bool {
        self.flags & FLAG_CONTROL != 0
    }

    /// True if the payload is voice.
    pub fn is_voice(&self) -> bool {
        self.flags & FLAG_VOICE != 0
    }
}

/// Generates a random message id (8 random bytes; ~2^32-message birthday
/// bound, ample for hand-PTT use).
pub fn random_message_id() -> u64 {
    rand::random()
}

/// Seals one frame: packs the header, encrypts the plaintext bound to it,
/// and returns the wire bytes (exactly plaintext len + FRAME_OVERHEAD).
fn seal_frame(key: &[u8; 32], header: &Header, plaintext: &[u8]) -> Result<Vec<u8>, AetrError> {
    let packed = header.pack();
    let ct = crypto::seal_chunk(key, &packed, header.message_id, header.chunk_index, plaintext)?;
    let mut frame = Vec::with_capacity(HEADER_LEN + ct.len());
    frame.extend_from_slice(&packed);
    frame.extend_from_slice(&ct);
    Ok(frame)
}

/// Sender-side cache kept per transmitted message for the optional ARQ
/// round: text keeps the full 2K shard pool, voice keeps its K spans.
pub struct TxCache {
    /// Message this cache belongs to.
    pub message_id: u64,
    /// Voice messages are repaired by exact retransmission, not parity.
    pub is_voice: bool,
    /// K: number of source chunks.
    pub source_count: usize,
    /// chunk_count as put in every header.
    pub chunk_count: u8,
    /// Text: all 2K plaintext shards (source + full parity pool).
    /// Voice: the K span plaintexts.
    pub shards: Vec<Vec<u8>>,
}

/// A message ready to transmit: frames in TX order plus the ARQ cache.
pub struct TxMessage {
    /// Message id used in every frame.
    pub message_id: u64,
    /// Wire frames in transmission order, each exactly the mode frame size.
    pub frames: Vec<Vec<u8>>,
    /// Cache for a later repair round.
    pub cache: TxCache,
}

/// Builds a text message: UTF-8 bytes behind a u16 length prefix, split
/// into K equal shards, parity pool encoded at N = 2K with the first
/// R = ceil(0.25 K) parity shards interleaved into the transmission.
pub fn build_text_message(
    key: &[u8; 32],
    mode: ModemMode,
    text: &str,
    message_id: u64,
) -> Result<TxMessage, AetrError> {
    let shard_len = mode.chunk_payload_bytes();
    let bytes = text.as_bytes();
    if bytes.len() > u16::MAX as usize {
        return Err(AetrError::TooLarge("text exceeds u16 length prefix".into()));
    }
    // Message body: length prefix + text, zero-padded to a shard multiple.
    let mut body = Vec::with_capacity(2 + bytes.len());
    body.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    body.extend_from_slice(bytes);
    let k = body.len().div_ceil(shard_len).max(1);
    if 2 * k > MAX_CHUNK_INDEX {
        return Err(AetrError::TooLarge(format!("text needs {k} chunks, max is 127")));
    }
    body.resize(k * shard_len, 0);
    let source: Vec<Vec<u8>> = body.chunks(shard_len).map(|c| c.to_vec()).collect();

    if k == 1 {
        let header = Header {
            message_id,
            chunk_index: 0,
            chunk_count: 1,
            flags: FLAG_LAST,
        };
        let frame = seal_frame(key, &header, &source[0])?;
        return Ok(TxMessage {
            message_id,
            frames: vec![frame],
            cache: TxCache {
                message_id,
                is_voice: false,
                source_count: 1,
                chunk_count: 1,
                shards: source,
            },
        });
    }

    let parity = fec::parity_pool(&source)?;
    let r = fec::repair_count(k);
    let total = k + r;
    let chunk_count = total as u8;

    // Round-robin interleave source and repair shards in TX order so a
    // cut-short reception window can still reach K of N.
    let mut frames = Vec::with_capacity(total);
    let (mut si, mut pi) = (0usize, 0usize);
    for t in 0..total {
        let take_parity = pi < r && (t + 1) * r >= (pi + 1) * total;
        let (idx, shard) = if take_parity {
            let idx = (k + pi) as u8;
            let shard = &parity[pi];
            pi += 1;
            (idx, shard)
        } else {
            let idx = si as u8;
            let shard = &source[si];
            si += 1;
            (idx, shard)
        };
        let mut flags = if idx as usize >= k { FLAG_REPAIR } else { 0 };
        if t == total - 1 {
            flags |= FLAG_LAST;
        }
        let header = Header { message_id, chunk_index: idx, chunk_count, flags };
        frames.push(seal_frame(key, &header, shard)?);
    }

    let mut shards = source;
    shards.extend(parity);
    Ok(TxMessage {
        message_id,
        frames,
        cache: TxCache {
            message_id,
            is_voice: false,
            source_count: k,
            chunk_count,
            shards,
        },
    })
}

/// Builds a voice message: the 48 kHz clip becomes K self-contained codec2
/// spans, one per chunk, with no erasure coding (missing spans degrade to
/// silence on the receiver).
pub fn build_voice_message(
    key: &[u8; 32],
    mode: ModemMode,
    pcm48k: &[f32],
    message_id: u64,
) -> Result<TxMessage, AetrError> {
    let shard_len = mode.chunk_payload_bytes();
    let spans = voice::encode_clip(pcm48k, shard_len)?;
    let k = spans.len();
    if k == 0 {
        return Err(AetrError::Voice("empty clip".into()));
    }
    if k > MAX_CHUNK_INDEX {
        return Err(AetrError::TooLarge(format!("clip needs {k} chunks, max is 254")));
    }
    let chunk_count = k as u8;
    let mut frames = Vec::with_capacity(k);
    for (i, span) in spans.iter().enumerate() {
        let mut flags = FLAG_VOICE;
        if i == k - 1 {
            flags |= FLAG_LAST;
        }
        let header = Header { message_id, chunk_index: i as u8, chunk_count, flags };
        frames.push(seal_frame(key, &header, span)?);
    }
    Ok(TxMessage {
        message_id,
        frames,
        cache: TxCache {
            message_id,
            is_voice: true,
            source_count: k,
            chunk_count,
            shards: spans,
        },
    })
}

/// An ARQ repair request: which shards of a message the receiver holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairRequest {
    /// The incomplete message.
    pub target: u64,
    /// Bitmask over chunk indices 0..255; bit set = shard received.
    pub received: [u8; 32],
}

impl RepairRequest {
    /// True if the bitmask marks this index as received.
    pub fn contains(&self, idx: usize) -> bool {
        idx < 256 && self.received[idx / 8] & (1 << (idx % 8)) != 0
    }

    /// Marks an index as received.
    fn set(&mut self, idx: usize) {
        if idx < 256 {
            self.received[idx / 8] |= 1 << (idx % 8);
        }
    }

    /// Serializes to the control-frame plaintext: target id LE + bitmask.
    fn to_plaintext(&self, shard_len: usize) -> Result<Vec<u8>, AetrError> {
        if shard_len < 40 {
            return Err(AetrError::Malformed("mode too small for control frame".into()));
        }
        let mut out = vec![0u8; shard_len];
        out[..8].copy_from_slice(&self.target.to_le_bytes());
        out[8..40].copy_from_slice(&self.received);
        Ok(out)
    }

    /// Parses a control-frame plaintext.
    fn from_plaintext(bytes: &[u8]) -> Result<RepairRequest, AetrError> {
        if bytes.len() < 40 {
            return Err(AetrError::Malformed("control payload shorter than 40 bytes".into()));
        }
        let mut id = [0u8; 8];
        id.copy_from_slice(&bytes[..8]);
        let mut mask = [0u8; 32];
        mask.copy_from_slice(&bytes[8..40]);
        Ok(RepairRequest { target: u64::from_le_bytes(id), received: mask })
    }
}

/// Seals a repair request into a single control frame (its own message id,
/// flags bit 3 set). Broadcast-safe: receivers without the target cached
/// simply ignore it.
pub fn build_control_frame(
    key: &[u8; 32],
    mode: ModemMode,
    request: &RepairRequest,
    message_id: u64,
) -> Result<Vec<u8>, AetrError> {
    let header = Header {
        message_id,
        chunk_index: 0,
        chunk_count: 1,
        flags: FLAG_CONTROL | FLAG_LAST,
    };
    seal_frame(key, &header, &request.to_plaintext(mode.chunk_payload_bytes())?)
}

/// Sender side of the ARQ round: given a repair request and the cached
/// message, produce just enough frames to close the gap. Text sends unused
/// parity shards from the pool (falling back to exact resends if the pool
/// runs dry); voice resends the exact missing spans.
pub fn build_repair_frames(
    key: &[u8; 32],
    cache: &TxCache,
    request: &RepairRequest,
) -> Result<Vec<Vec<u8>>, AetrError> {
    if request.target != cache.message_id {
        return Err(AetrError::Malformed("repair request targets a different message".into()));
    }
    let k = cache.source_count;
    let mut frames = Vec::new();

    if cache.is_voice {
        // Exact retransmission of missing spans; identical header + nonce
        // reproduce the original ciphertext byte for byte.
        for idx in 0..k {
            if !request.contains(idx) {
                let mut flags = FLAG_VOICE;
                if idx == k - 1 {
                    flags |= FLAG_LAST;
                }
                let header = Header {
                    message_id: cache.message_id,
                    chunk_index: idx as u8,
                    chunk_count: cache.chunk_count,
                    flags,
                };
                frames.push(seal_frame(key, &header, &cache.shards[idx])?);
            }
        }
        return Ok(frames);
    }

    if k == 1 {
        // Single-chunk text: nothing in the pool, resend the chunk itself.
        if !request.contains(0) {
            let header = Header {
                message_id: cache.message_id,
                chunk_index: 0,
                chunk_count: 1,
                flags: FLAG_LAST,
            };
            frames.push(seal_frame(key, &header, &cache.shards[0])?);
        }
        return Ok(frames);
    }

    // MDS: the receiver needs K distinct shards; count what it holds.
    let held = (0..2 * k).filter(|&i| request.contains(i)).count();
    if held >= k {
        return Ok(frames);
    }
    let mut needed = k - held;

    // Prefer parity shards never transmitted (index >= K + R).
    let r = fec::repair_count(k);
    let mut candidates: Vec<usize> = ((k + r)..(2 * k)).filter(|&i| !request.contains(i)).collect();
    // Then fall back to resending whatever the receiver reports missing.
    candidates.extend((0..(k + r)).filter(|&i| !request.contains(i)));

    for idx in candidates {
        if needed == 0 {
            break;
        }
        let flags = if idx >= k { FLAG_REPAIR } else { 0 };
        let header = Header {
            message_id: cache.message_id,
            chunk_index: idx as u8,
            chunk_count: cache.chunk_count,
            flags,
        };
        frames.push(seal_frame(key, &header, &cache.shards[idx])?);
        needed -= 1;
    }
    Ok(frames)
}

/// A fully reassembled (or failed) incoming message.
#[derive(Debug, Clone, PartialEq)]
pub enum RxEvent {
    /// Complete text message.
    Text {
        /// Message id it arrived under.
        message_id: u64,
        /// Decoded UTF-8 text.
        text: String,
    },
    /// Voice clip, possibly with silent gaps where spans were lost.
    Voice {
        /// Message id it arrived under.
        message_id: u64,
        /// 48 kHz mono PCM ready to play.
        pcm48k: Vec<f32>,
        /// Span indices that were missing and filled with silence.
        missing_spans: Vec<u32>,
    },
    /// An ARQ control frame addressed at some sender on the channel.
    Control {
        /// The parsed repair request.
        request: RepairRequest,
    },
    /// A message that timed out or could not be decoded.
    Failed {
        /// Message id of the failed message.
        message_id: u64,
        /// Why it failed.
        reason: String,
    },
}

/// Reassembly state for one in-flight message.
struct Partial {
    voice: bool,
    chunk_count: u8,
    shards: BTreeMap<u8, Vec<u8>>,
    first_seen: Instant,
}

/// Receiver-side reassembly: decrypts arriving frames, tracks per-message
/// state, recovers text via the RS pool, and finalizes voice (with silence
/// fill) on completion or timeout.
pub struct Reassembler {
    key: [u8; 32],
    partials: HashMap<u64, Partial>,
    delivered: HashSet<u64>,
}

impl Reassembler {
    /// Creates a reassembler bound to a session key.
    pub fn new(key: [u8; 32]) -> Self {
        Reassembler { key, partials: HashMap::new(), delivered: HashSet::new() }
    }

    /// Feeds one raw frame (header + ciphertext) as it comes off the modem.
    /// Returns a completed event when this frame finished a message. A frame
    /// that fails authentication returns `Err(AuthFailed)` and changes no
    /// state — with a wrong passphrase every chunk lands here.
    pub fn push_frame(&mut self, frame: &[u8], now: Instant) -> Result<Option<RxEvent>, AetrError> {
        if frame.len() < FRAME_OVERHEAD + 1 {
            return Err(AetrError::Malformed("frame shorter than header + tag".into()));
        }
        let header = Header::unpack(frame)?;
        let mut packed = [0u8; HEADER_LEN];
        packed.copy_from_slice(&frame[..HEADER_LEN]);
        let plaintext = crypto::open_chunk(
            &self.key,
            &packed,
            header.message_id,
            header.chunk_index,
            &frame[HEADER_LEN..],
        )?;

        if header.is_control() {
            return Ok(Some(RxEvent::Control { request: RepairRequest::from_plaintext(&plaintext)? }));
        }
        if self.delivered.contains(&header.message_id) {
            return Ok(None); // Late repair for something already complete.
        }
        if header.chunk_count == 0 {
            return Err(AetrError::Malformed("chunk_count of zero".into()));
        }

        // Validate the index against the message geometry.
        let voice = header.is_voice();
        let index_limit = if voice {
            header.chunk_count as usize
        } else {
            let k = fec::source_count_from_total(header.chunk_count as usize)
                .ok_or_else(|| AetrError::Malformed("impossible chunk_count".into()))?;
            2 * k
        };
        if header.chunk_index as usize >= index_limit {
            return Err(AetrError::Malformed("chunk_index out of range".into()));
        }

        let partial = self.partials.entry(header.message_id).or_insert_with(|| Partial {
            voice,
            chunk_count: header.chunk_count,
            shards: BTreeMap::new(),
            first_seen: now,
        });
        if partial.voice != voice || partial.chunk_count != header.chunk_count {
            return Err(AetrError::Malformed("conflicting headers within a message".into()));
        }
        partial.shards.entry(header.chunk_index).or_insert(plaintext);

        self.try_complete(header.message_id)
    }

    /// Attempts to finalize a message whose shard set may now be sufficient.
    fn try_complete(&mut self, message_id: u64) -> Result<Option<RxEvent>, AetrError> {
        let Some(partial) = self.partials.get(&message_id) else {
            return Ok(None);
        };
        if partial.voice {
            if partial.shards.len() < partial.chunk_count as usize {
                return Ok(None);
            }
        } else {
            let k = fec::source_count_from_total(partial.chunk_count as usize)
                .ok_or_else(|| AetrError::Malformed("impossible chunk_count".into()))?;
            if partial.shards.len() < k {
                return Ok(None);
            }
        }
        let partial = self.partials.remove(&message_id).expect("checked above");
        self.delivered.insert(message_id);
        Ok(Some(Self::finalize(message_id, partial)?))
    }

    /// Decodes a message from its collected shards. Voice tolerates missing
    /// spans (silence fill); text requires K distinct shards.
    fn finalize(message_id: u64, partial: Partial) -> Result<RxEvent, AetrError> {
        if partial.voice {
            let count = partial.chunk_count as usize;
            let mut spans: Vec<Option<Vec<u8>>> = vec![None; count];
            for (idx, shard) in partial.shards {
                spans[idx as usize] = Some(shard);
            }
            let (pcm48k, missing_spans) = voice::decode_clip(&spans)?;
            return Ok(RxEvent::Voice { message_id, pcm48k, missing_spans });
        }

        let k = fec::source_count_from_total(partial.chunk_count as usize)
            .ok_or_else(|| AetrError::Malformed("impossible chunk_count".into()))?;
        let source: Vec<Vec<u8>> = if k == 1 {
            match partial.shards.into_iter().next() {
                Some((0, shard)) => vec![shard],
                _ => return Err(AetrError::Malformed("single-chunk message without chunk 0".into())),
            }
        } else {
            let mut pool: Vec<Option<Vec<u8>>> = vec![None; 2 * k];
            for (idx, shard) in partial.shards {
                pool[idx as usize] = Some(shard);
            }
            fec::reconstruct_source(k, &mut pool)?;
            pool.into_iter()
                .take(k)
                .map(|s| s.ok_or_else(|| AetrError::Fec("reconstruction left a hole".into())))
                .collect::<Result<_, _>>()?
        };

        let body: Vec<u8> = source.concat();
        if body.len() < 2 {
            return Err(AetrError::Malformed("text body shorter than length prefix".into()));
        }
        let len = u16::from_le_bytes([body[0], body[1]]) as usize;
        if body.len() < 2 + len {
            return Err(AetrError::Malformed("text length prefix exceeds body".into()));
        }
        let text = String::from_utf8(body[2..2 + len].to_vec())
            .map_err(|_| AetrError::Malformed("text is not valid UTF-8".into()))?;
        Ok(RxEvent::Text { message_id, text })
    }

    /// Builds a repair request for an incomplete message, or None if the
    /// message is unknown (or already delivered).
    pub fn build_repair_request(&self, message_id: u64) -> Option<RepairRequest> {
        let partial = self.partials.get(&message_id)?;
        let mut request = RepairRequest { target: message_id, received: [0u8; 32] };
        for &idx in partial.shards.keys() {
            request.set(idx as usize);
        }
        Some(request)
    }

    /// Sweeps messages older than `timeout`. Voice finalizes with silence
    /// gaps; text becomes a Failed event. Call periodically (per-message
    /// timeout state machine).
    pub fn take_expired(&mut self, now: Instant, timeout: Duration) -> Vec<RxEvent> {
        let expired: Vec<u64> = self
            .partials
            .iter()
            .filter(|(_, p)| now.duration_since(p.first_seen) >= timeout)
            .map(|(&id, _)| id)
            .collect();
        let mut events = Vec::new();
        for id in expired {
            let partial = match self.partials.remove(&id) {
                Some(p) => p,
                None => continue,
            };
            self.delivered.insert(id);
            if partial.voice {
                match Self::finalize(id, partial) {
                    Ok(ev) => events.push(ev),
                    Err(e) => events.push(RxEvent::Failed { message_id: id, reason: e.to_string() }),
                }
            } else {
                let k = fec::source_count_from_total(partial.chunk_count as usize).unwrap_or(0);
                events.push(RxEvent::Failed {
                    message_id: id,
                    reason: format!(
                        "timed out with {} of {} shards",
                        partial.shards.len(),
                        k.max(1)
                    ),
                });
            }
        }
        events
    }

    /// Number of messages still being assembled.
    pub fn pending(&self) -> usize {
        self.partials.len()
    }

    /// Snapshot of in-flight messages for progress reporting:
    /// `(message_id, distinct shards held, shards needed to complete,
    /// is_voice)`. For text the need is K source-equivalents (MDS); for
    /// voice it is the span count.
    pub fn progress(&self) -> Vec<(u64, usize, usize, bool)> {
        self.partials
            .iter()
            .map(|(&id, p)| {
                let needed = if p.voice {
                    p.chunk_count as usize
                } else {
                    fec::source_count_from_total(p.chunk_count as usize)
                        .unwrap_or(p.chunk_count as usize)
                };
                (id, p.shards.len(), needed, p.voice)
            })
            .collect()
    }
}
