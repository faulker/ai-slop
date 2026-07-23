//! M1 acceptance tests: end-to-end loopback over a simulated noisy channel,
//! RS recovery, the ARQ repair round, voice degradation, auth failure, and
//! known-answer vectors. DSP-heavy tests are meant to run in release
//! (`cargo test -p aetr-core --release`).

use crate::channel;
use crate::crypto;
use crate::fec;
use crate::frame::{self, Header, Reassembler, RxEvent};
use crate::modem::{Modem, ModemMode, OfdmModem, OfdmRx};
use crate::voice;
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Shared session key so the ~100 ms Argon2id run happens once per binary.
fn test_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| crypto::derive_key("correct horse battery staple").expect("kdf"))
}

/// Lowercase hex helper for the known-answer vectors.
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Concatenates bursts with silence gaps and lead-in/out, so the decoder
/// sees realistic squelch-style spacing. `drop` lists burst indices to omit
/// entirely (whole-frame drops).
fn splice_bursts(bursts: &[Vec<f32>], drop: &[usize]) -> Vec<f32> {
    let gap = vec![0f32; 4800];
    let lead = vec![0f32; 24000];
    let mut out = lead.clone();
    for (i, b) in bursts.iter().enumerate() {
        if drop.contains(&i) {
            continue;
        }
        out.extend_from_slice(b);
        out.extend_from_slice(&gap);
    }
    out.extend_from_slice(&lead);
    out
}

/// Runs PCM through a fresh receiver and returns all recovered frames.
fn receive_all(pcm: &[f32]) -> Vec<Vec<u8>> {
    let mut rx = OfdmRx::new().expect("rx alloc");
    rx.feed(pcm).expect("rx feed")
}

/// Encodes every frame of a message into bursts.
fn modulate(mode: ModemMode, frames: &[Vec<u8>]) -> Vec<Vec<f32>> {
    let modem = OfdmModem;
    frames
        .iter()
        .map(|f| modem.encode_frame(mode, f).expect("encode burst"))
        .collect()
}

/// Pushes raw frames into a reassembler, panicking on any error, returning
/// the first completed event if any.
fn push_frames(rs: &mut Reassembler, frames: &[Vec<u8>]) -> Option<RxEvent> {
    let now = Instant::now();
    let mut event = None;
    for f in frames {
        if let Some(ev) = rs.push_frame(f, now).expect("push frame") {
            event = Some(ev);
        }
    }
    event
}

#[test]
fn shim_smoke_sizes_agree() {
    // M0 smoke test: the C++ shim is linked and agrees with the Rust side
    // on frame geometry.
    for mode in [ModemMode::B85, ModemMode::B128, ModemMode::B170] {
        assert_eq!(crate::modem::shim_payload_bytes(mode), mode.frame_bytes());
    }
    assert_eq!(crate::modem::burst_samples(), 7 * 8640);
}

#[test]
fn modem_loopback_clean_all_modes() {
    for mode in [ModemMode::B85, ModemMode::B128, ModemMode::B170] {
        let payload: Vec<u8> = (0..mode.frame_bytes() as u32).map(|i| (i * 7 + 3) as u8).collect();
        let modem = OfdmModem;
        let burst = modem.encode_frame(mode, &payload).expect("encode");
        assert_eq!(burst.len(), crate::modem::burst_samples());
        let pcm = splice_bursts(&[burst], &[]);
        let frames = receive_all(&pcm);
        assert_eq!(frames.len(), 1, "expected one decoded frame in {mode:?}");
        assert_eq!(frames[0], payload, "payload mismatch in {mode:?}");
    }
}

#[test]
fn modem_loopback_awgn_10db() {
    let mode = ModemMode::B170;
    let payload: Vec<u8> = (0..170u32).map(|i| (i * 13 + 1) as u8).collect();
    let modem = OfdmModem;
    let burst = modem.encode_frame(mode, &payload).expect("encode");
    let mut pcm = splice_bursts(&[burst], &[]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0001);
    channel::awgn(&mut pcm, 10.0, &mut rng);
    let frames = receive_all(&pcm);
    assert_eq!(frames.len(), 1, "burst should survive 10 dB SNR");
    assert_eq!(frames[0], payload);
}

#[test]
fn modem_loopback_skew_scale_dc() {
    let mode = ModemMode::B85;
    let payload: Vec<u8> = (0..85u32).map(|i| (i * 31 + 5) as u8).collect();
    let modem = OfdmModem;
    let burst = modem.encode_frame(mode, &payload).expect("encode");
    let clean = splice_bursts(&[burst], &[]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0002);
    // +200 ppm clock skew, 0.6x level, 2% DC offset, 15 dB SNR.
    let pcm = channel::impaired(&clean, 15.0, 200.0, 0.6, 0.02, &mut rng);
    let frames = receive_all(&pcm);
    assert_eq!(frames.len(), 1, "burst should survive skew/scale/DC");
    assert_eq!(frames[0], payload);
}

#[test]
fn text_roundtrip_single_chunk() {
    let key = test_key();
    let mode = ModemMode::B170;
    let text = "hello over fm";
    let tx = frame::build_text_message(key, mode, text, 0x1001).expect("build");
    assert_eq!(tx.frames.len(), 1);
    assert_eq!(tx.frames[0].len(), mode.frame_bytes());

    let bursts = modulate(mode, &tx.frames);
    let mut pcm = splice_bursts(&bursts, &[]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0003);
    channel::awgn(&mut pcm, 25.0, &mut rng);

    let mut rs = Reassembler::new(*key);
    let event = push_frames(&mut rs, &receive_all(&pcm));
    match event {
        Some(RxEvent::Text { message_id, text: t }) => {
            assert_eq!(message_id, 0x1001);
            assert_eq!(t, text);
        }
        other => panic!("expected text event, got {other:?}"),
    }
}

/// Builds a text of exactly `len` ASCII bytes with distinctive content.
fn long_text(len: usize) -> String {
    let base = "the quick brown fox jumps over the lazy dog 0123456789 ";
    base.chars().cycle().take(len).collect()
}

#[test]
fn text_roundtrip_ten_chunks() {
    let key = test_key();
    let mode = ModemMode::B170;
    // 1400 bytes -> (2 + 1400) / 143 -> K = 10 source chunks, R = 3.
    let text = long_text(1400);
    let tx = frame::build_text_message(key, mode, &text, 0x1002).expect("build");
    assert_eq!(tx.cache.source_count, 10);
    assert_eq!(tx.frames.len(), 13);

    let bursts = modulate(mode, &tx.frames);
    let mut pcm = splice_bursts(&bursts, &[]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0004);
    channel::awgn(&mut pcm, 25.0, &mut rng);

    let mut rs = Reassembler::new(*key);
    let event = push_frames(&mut rs, &receive_all(&pcm));
    match event {
        Some(RxEvent::Text { text: t, .. }) => assert_eq!(t, text),
        other => panic!("expected text event, got {other:?}"),
    }
}

#[test]
fn text_ten_chunks_two_drops_recovered_by_rs() {
    let key = test_key();
    let mode = ModemMode::B170;
    let text = long_text(1400);
    let tx = frame::build_text_message(key, mode, &text, 0x1003).expect("build");
    assert_eq!(tx.frames.len(), 13);

    let bursts = modulate(mode, &tx.frames);
    // Drop two whole bursts; 11 of 13 distinct shards still exceed K = 10.
    let mut pcm = splice_bursts(&bursts, &[1, 6]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0005);
    channel::awgn(&mut pcm, 25.0, &mut rng);

    let mut rs = Reassembler::new(*key);
    let frames = receive_all(&pcm);
    assert_eq!(frames.len(), 11);
    let event = push_frames(&mut rs, &frames);
    match event {
        Some(RxEvent::Text { text: t, .. }) => assert_eq!(t, text),
        other => panic!("expected text event, got {other:?}"),
    }
}

#[test]
fn arq_round_recovers_four_drops() {
    let key = test_key();
    let mode = ModemMode::B170;
    let text = long_text(1400);
    let tx = frame::build_text_message(key, mode, &text, 0x1004).expect("build");
    let k = tx.cache.source_count;
    assert_eq!(k, 10);

    // Drop 4 source chunks — beyond R = 3, so the upfront burst can't finish.
    let dropped: [u8; 4] = [0, 2, 4, 6];
    let mut rs = Reassembler::new(*key);
    let now = Instant::now();
    let mut completed = None;
    for f in &tx.frames {
        let h = Header::unpack(f).expect("header");
        if dropped.contains(&h.chunk_index) {
            continue;
        }
        if let Some(ev) = rs.push_frame(f, now).expect("push") {
            completed = Some(ev);
        }
    }
    assert!(completed.is_none(), "9 of 10 shards must not complete");
    assert_eq!(rs.pending(), 1);

    // Receiver builds the control frame from its own state...
    let request = rs.build_repair_request(0x1004).expect("repair request");
    let control = frame::build_control_frame(key, mode, &request, 0x2004).expect("control");
    assert_eq!(control.len(), mode.frame_bytes());

    // ...the sender decodes it (it's a normal encrypted frame)...
    let mut sender_rs = Reassembler::new(*key);
    let parsed = match sender_rs.push_frame(&control, now).expect("control push") {
        Some(RxEvent::Control { request }) => request,
        other => panic!("expected control event, got {other:?}"),
    };
    assert_eq!(parsed, request);

    // ...and answers with cached parity from the pool (never-transmitted
    // shards, indices >= K + R).
    let repairs = frame::build_repair_frames(key, &tx.cache, &parsed).expect("repairs");
    assert_eq!(repairs.len(), 1, "receiver holds 9 shards, needs exactly 1");
    let rh = Header::unpack(&repairs[0]).expect("repair header");
    assert!(rh.chunk_index as usize >= k + fec::repair_count(k), "fresh pool parity expected");

    let event = push_frames(&mut rs, &repairs);
    match event {
        Some(RxEvent::Text { text: t, .. }) => assert_eq!(t, text),
        other => panic!("expected completed text, got {other:?}"),
    }
}

/// A 440 Hz test tone at 48 kHz.
fn tone(seconds: f32) -> Vec<f32> {
    let n = (seconds * 48000.0) as usize;
    (0..n)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin())
        .collect()
}

#[test]
fn voice_roundtrip_over_modem() {
    let key = test_key();
    let mode = ModemMode::B170;
    let clip = tone(1.0);
    let tx = frame::build_voice_message(key, mode, &clip, 0x3001).expect("build");
    assert!(tx.frames.len() >= 2, "1 s at 1200 bps spans two chunks");

    let bursts = modulate(mode, &tx.frames);
    let mut pcm = splice_bursts(&bursts, &[]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0006);
    channel::awgn(&mut pcm, 25.0, &mut rng);

    let mut rs = Reassembler::new(*key);
    let event = push_frames(&mut rs, &receive_all(&pcm));
    match event {
        Some(RxEvent::Voice { message_id, pcm48k, missing_spans }) => {
            assert_eq!(message_id, 0x3001);
            assert!(missing_spans.is_empty());
            // codec2 is lossy; just require real signal energy out.
            let energy: f32 = pcm48k.iter().map(|s| s * s).sum();
            assert!(energy > 1.0, "decoded clip should carry energy, got {energy}");
        }
        other => panic!("expected voice event, got {other:?}"),
    }
}

#[test]
fn voice_dropped_chunk_leaves_silence_gap_at_offset() {
    let key = test_key();
    let mode = ModemMode::B170;
    let clip = tone(2.5); // 63 codec2 frames -> spans of 23 + 23 + 17.
    let tx = frame::build_voice_message(key, mode, &clip, 0x3002).expect("build");
    assert_eq!(tx.frames.len(), 3);

    let mut rs = Reassembler::new(*key);
    let t0 = Instant::now();
    for (i, f) in tx.frames.iter().enumerate() {
        if i == 1 {
            continue; // Drop the middle span.
        }
        let ev = rs.push_frame(f, t0).expect("push");
        assert!(ev.is_none(), "incomplete voice must wait for timeout");
    }

    // The per-message timeout finalizes the clip with a silence gap.
    let events = rs.take_expired(t0 + Duration::from_secs(31), Duration::from_secs(30));
    assert_eq!(events.len(), 1);
    match &events[0] {
        RxEvent::Voice { pcm48k, missing_spans, .. } => {
            assert_eq!(missing_spans, &vec![1u32]);
            // Span geometry: 23 frames * 320 samples * 6x upsample each.
            let span = 23 * 320 * 6;
            let margin = 2400; // FIR spill at the gap edges.
            let e = |range: std::ops::Range<usize>| -> f32 {
                pcm48k[range].iter().map(|s| s * s).sum()
            };
            let e0 = e(margin..span - margin);
            let egap = e(span + margin..2 * span - margin);
            let e2 = e(2 * span + margin..pcm48k.len().min(3 * span) - margin);
            assert!(e0 > 1.0, "span 0 should have audio, energy {e0}");
            assert!(e2 > 1.0, "span 2 should have audio, energy {e2}");
            assert!(egap < e0 * 1e-4, "gap should be silent: {egap} vs {e0}");
        }
        other => panic!("expected voice event, got {other:?}"),
    }
}

#[test]
fn wrong_passphrase_fails_every_chunk() {
    let key = test_key();
    let wrong = crypto::derive_key("wrong horse wrong staple").expect("kdf");
    let mode = ModemMode::B170;
    let tx = frame::build_text_message(key, mode, &long_text(500), 0x4001).expect("build");

    let mut rs = Reassembler::new(wrong);
    let now = Instant::now();
    for f in &tx.frames {
        match rs.push_frame(f, now) {
            Err(crate::AetrError::AuthFailed) => {}
            other => panic!("expected AuthFailed, got {other:?}"),
        }
    }
    assert_eq!(rs.pending(), 0, "no partial state may accumulate");
    assert!(rs.take_expired(now + Duration::from_secs(60), Duration::from_secs(30)).is_empty());
}

#[test]
fn reassembler_rejects_tampered_frame() {
    let key = test_key();
    let mode = ModemMode::B170;
    let tx = frame::build_text_message(key, mode, "tamper me", 0x4002).expect("build");
    let mut bad = tx.frames[0].clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    let mut rs = Reassembler::new(*key);
    assert_eq!(rs.push_frame(&bad, Instant::now()), Err(crate::AetrError::AuthFailed));
}

#[test]
fn fec_transmit_count_is_invertible() {
    for k in 1..=127usize {
        let total = fec::transmit_count(k);
        assert_eq!(
            fec::source_count_from_total(total),
            Some(k),
            "K={k} total={total} must invert"
        );
    }
}

#[test]
fn resampler_preserves_tone() {
    // 1 kHz sine through 48k -> 8k -> 48k should keep most of its energy.
    let n = 48000;
    let sine: Vec<f32> = (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin())
        .collect();
    let down = voice::downsample_48k_to_8k(&sine);
    assert_eq!(down.len(), n / 6);
    let up = voice::upsample_8k_to_48k(&down);
    let mid = &up[4800..up.len() - 4800];
    let energy: f32 = mid.iter().map(|s| s * s).sum::<f32>() / mid.len() as f32;
    assert!((energy - 0.5).abs() < 0.05, "tone energy after resample: {energy}");
}

// --- Golden cross-platform vectors -------------------------------------
//
// The golden wavs are deterministic: fixed passphrase, fixed message_id,
// derived nonces, deterministic RS parity and modem encode. First run
// writes testdata/golden_*.wav; later runs assert the encoder hasn't
// drifted and that the stored file still decodes. The macOS app
// (`Aetr --selftest` / `--decode-golden`) and the Android JVM test
// (GoldenWavTest.kt) decode the same files to prove byte-identical
// behavior across platforms. If a legitimate encoder change breaks the
// drift check, delete the wav files to regenerate them everywhere.

/// Passphrase shared by all golden vectors (same as vectors.json).
pub const GOLDEN_PASSPHRASE: &str = "correct horse battery staple";
/// Plaintext inside golden_text.wav (single chunk in B170 mode).
pub const GOLDEN_TEXT: &str =
    "aetr golden vector: the quick brown fox jumps over the lazy dog 0123456789";
/// Fixed message_id of golden_text.wav.
pub const GOLDEN_TEXT_ID: u64 = 0xAE72_0001;
/// Fixed message_id of golden_voice.wav (1 s 440 Hz tone, two chunks).
pub const GOLDEN_VOICE_ID: u64 = 0xAE72_0002;

/// Absolute path of a file in testdata/.
fn testdata_path(name: &str) -> String {
    format!("{}/../testdata/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// Converts f32 PCM to 16-bit with clamping (the golden wav sample format).
fn f32_to_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect()
}

/// Converts 16-bit wav samples back to f32 for the receiver.
fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 32768.0).collect()
}

/// Writes a canonical 48 kHz mono 16-bit PCM WAV file.
fn write_wav_i16(path: &str, samples: &[i16]) {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&48_000u32.to_le_bytes());
    out.extend_from_slice(&96_000u32.to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).expect("write golden wav");
}

/// Reads a canonical 48 kHz mono 16-bit PCM WAV (walking RIFF chunks).
fn read_wav_i16(path: &str) -> Vec<i16> {
    let bytes = std::fs::read(path).expect("read golden wav");
    assert!(bytes.len() > 44 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE");
    let mut pos = 12;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..pos + 8 + len];
        match id {
            b"fmt " => {
                assert_eq!(u16::from_le_bytes(body[0..2].try_into().unwrap()), 1, "PCM");
                assert_eq!(u16::from_le_bytes(body[2..4].try_into().unwrap()), 1, "mono");
                assert_eq!(u32::from_le_bytes(body[4..8].try_into().unwrap()), 48_000);
                assert_eq!(u16::from_le_bytes(body[14..16].try_into().unwrap()), 16, "16-bit");
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + len + (len & 1); // chunks are word-aligned
    }
    data.expect("data chunk")
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

/// Generates (first run) or verifies (later runs) a golden wav, then
/// decodes the stored file and returns the resulting event.
fn golden_roundtrip(name: &str, frames: &[Vec<u8>]) -> RxEvent {
    let bursts = modulate(ModemMode::B170, frames);
    let samples = f32_to_i16(&splice_bursts(&bursts, &[]));
    let path = testdata_path(name);
    if !std::path::Path::new(&path).exists() {
        write_wav_i16(&path, &samples);
    }
    let stored = read_wav_i16(&path);
    assert_eq!(
        stored, samples,
        "{name} drifted from the current encoder; delete testdata/{name} to regenerate"
    );
    let mut rs = Reassembler::new(*test_key());
    push_frames(&mut rs, &receive_all(&i16_to_f32(&stored))).expect("golden wav must decode")
}

#[test]
fn golden_text_wav_cross_platform() {
    let tx = frame::build_text_message(test_key(), ModemMode::B170, GOLDEN_TEXT, GOLDEN_TEXT_ID)
        .expect("build");
    assert_eq!(tx.frames.len(), 1, "golden text must stay single-chunk");
    match golden_roundtrip("golden_text.wav", &tx.frames) {
        RxEvent::Text { message_id, text } => {
            assert_eq!(message_id, GOLDEN_TEXT_ID);
            assert_eq!(text, GOLDEN_TEXT);
        }
        other => panic!("expected text event, got {other:?}"),
    }
}

#[test]
fn golden_voice_wav_cross_platform() {
    let clip = tone(1.0);
    let tx = frame::build_voice_message(test_key(), ModemMode::B170, &clip, GOLDEN_VOICE_ID)
        .expect("build");
    match golden_roundtrip("golden_voice.wav", &tx.frames) {
        RxEvent::Voice { message_id, pcm48k, missing_spans } => {
            assert_eq!(message_id, GOLDEN_VOICE_ID);
            assert!(missing_spans.is_empty());
            let energy: f32 = pcm48k.iter().map(|s| s * s).sum();
            assert!(energy > 1.0, "golden voice clip should carry energy, got {energy}");
        }
        other => panic!("expected voice event, got {other:?}"),
    }
}

#[test]
fn known_answer_vectors() {
    // KDF, nonce, and header known answers, persisted to testdata/vectors.json.
    // First run writes the file; later runs verify against it.
    let passphrase = "correct horse battery staple";
    let key = test_key();

    let nonce_a = crypto::nonce_for(0x1122334455667788, 0x99);
    let nonce_b = crypto::nonce_for(1, 0);
    let header = Header {
        message_id: 0x0102030405060708,
        chunk_index: 3,
        chunk_count: 13,
        flags: frame::FLAG_REPAIR | frame::FLAG_LAST,
    };

    let computed = serde_json::json!({
        "kdf": {
            "algorithm": "argon2id",
            "version": 19,
            "m_cost_kib": crypto::KDF_M_COST_KIB,
            "t_cost": crypto::KDF_T_COST,
            "p_cost": crypto::KDF_P_COST,
            "salt": String::from_utf8(crypto::KDF_SALT.to_vec()).unwrap(),
            "passphrase": passphrase,
            "key_hex": to_hex(key),
        },
        "nonces": [
            {
                "message_id": 0x1122334455667788u64,
                "chunk_index": 0x99,
                "nonce_hex": to_hex(&nonce_a),
            },
            {
                "message_id": 1,
                "chunk_index": 0,
                "nonce_hex": to_hex(&nonce_b),
            },
        ],
        "headers": [
            {
                "message_id": 0x0102030405060708u64,
                "chunk_index": 3,
                "chunk_count": 13,
                "flags": header.flags,
                "packed_hex": to_hex(&header.pack()),
            },
        ],
    });

    // Structural known answers that hold regardless of the file.
    assert_eq!(to_hex(&nonce_a), "887766554433221199000000000000000000000000000000");
    assert_eq!(to_hex(&nonce_b), "010000000000000000000000000000000000000000000000");
    assert_eq!(to_hex(&header.pack()), "0807060504030201030d05");

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../testdata/vectors.json");
    match std::fs::read_to_string(path) {
        Ok(existing) => {
            let stored: serde_json::Value = serde_json::from_str(&existing).expect("valid json");
            assert_eq!(stored, computed, "known-answer vectors drifted");
        }
        Err(_) => {
            std::fs::write(path, serde_json::to_string_pretty(&computed).unwrap())
                .expect("write vectors.json");
        }
    }
}
