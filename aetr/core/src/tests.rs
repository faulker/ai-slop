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

// Analog-path margin. These exist to answer "which impairment actually
// breaks a link", so a field capture's measurements can be turned into a
// verdict instead of a guess. Each one records a measured result, not an
// assumption; the surprises are documented where they occur.

/// One B85 burst spliced with the usual lead-in, gaps, and tail.
fn margin_burst() -> (Vec<u8>, Vec<f32>) {
    let payload: Vec<u8> = (0..85u32).map(|i| (i * 7 + 1) as u8).collect();
    let burst = OfdmModem.encode_frame(ModemMode::B85, &payload).expect("encode");
    let pcm = splice_bursts(&[burst], &[]);
    (payload, pcm)
}

#[test]
fn survives_severe_hard_clipping() {
    // Measured: the burst still decodes clipped 20 dB into its own peak,
    // i.e. driven hard into a limiter. Clipping looks like the obvious
    // suspect for an 8.3 dB PAPR signal, and it is not one. Differential
    // QPSK cares about phase, which hard clipping largely preserves.
    let (payload, clean) = margin_burst();
    let peak = clean.iter().fold(0f32, |m, s| m.max(s.abs()));
    for ceiling in [peak * 0.5, peak * 0.2, peak * 0.1] {
        let mut pcm = clean.clone();
        channel::clip(&mut pcm, ceiling);
        let frames = receive_all(&pcm);
        assert_eq!(
            frames.first(),
            Some(&payload),
            "clipping at {ceiling:.3} ({:.0} dB below peak) should still decode",
            20.0 * (ceiling / peak).log10()
        );
    }
}

#[test]
fn survives_radio_bandlimiting_and_deemphasis() {
    // Measured: a 300-3000 Hz radio filter is a non-event, and so is a much
    // tighter 1100-3000 Hz one that removes a large share of the lower
    // carriers. The polar code plus per-carrier erasure absorbs the loss.
    // FM de-emphasis tilt across the band is likewise survivable.
    let (payload, clean) = margin_burst();
    for (lo, hi) in [(300.0, 3000.0), (300.0, 2400.0), (1100.0, 3000.0)] {
        let pcm = channel::bandlimit(&clean, lo, hi);
        assert_eq!(
            receive_all(&pcm).first(),
            Some(&payload),
            "a {lo}-{hi} Hz channel should still decode"
        );
    }
    assert_eq!(
        receive_all(&channel::deemphasis(&clean)).first(),
        Some(&payload),
        "FM de-emphasis tilt should still decode"
    );
}

#[test]
fn syllabic_agc_destroys_the_payload() {
    // Measured, and the one impairment here that is actually fatal.
    //
    // The payload is differentially encoded across symbols: each carrier is
    // divided by its own value in the previous symbol, 180 ms earlier. That
    // cancels any gain which is constant over a symbol pair, so a very fast
    // AGC (effectively a limiter) and a very slow one both pass. A gain that
    // moves *on the symbol timescale* does not cancel, and the constellation
    // comes apart.
    //
    // The failing window below (attack ~7-21 ms, release ~70-210 ms) is
    // ordinary speech-compressor tuning, which is what a phone's
    // voice-communication audio path applies. That makes routing a burst
    // through MODE_IN_COMMUNICATION an actively destructive thing to do.
    let (_, clean) = margin_burst();
    for (attack, release) in [(0.003, 0.0003), (0.001, 0.0001)] {
        let pcm = channel::agc(&clean, 0.15, attack, release);
        assert!(
            receive_all(&pcm).is_empty(),
            "AGC at attack={attack} release={release} is expected to break decode; \
             if this now passes, the demodulator got more tolerant and the \
             voice-path routing guidance should be revisited"
        );
    }
}

#[test]
fn agc_outside_the_symbol_timescale_is_harmless() {
    // The other half of the same finding: gain movement far faster or far
    // slower than one symbol cancels out. Without this, the test above could
    // pass for the wrong reason (any AGC breaking anything).
    let (payload, clean) = margin_burst();
    for (attack, release) in [(0.01, 0.001), (0.0001, 0.00001)] {
        let pcm = channel::agc(&clean, 0.15, attack, release);
        assert_eq!(
            receive_all(&pcm).first(),
            Some(&payload),
            "AGC at attack={attack} release={release} should be survivable"
        );
    }
}

#[test]
fn quiet_input_has_a_hard_floor() {
    // The receiver has no AGC of its own and the sync metric clamps its power
    // term (min_R in schmidl_cox.hh), so below roughly -57 dBFS RMS nothing
    // syncs at all and every counter reads zero. That is the same signature
    // as a dead audio route, which is exactly why the input meter needs an
    // RMS readout and not just a peak one.
    let (payload, clean) = margin_burst();

    let mut loud = clean.clone();
    channel::scale(&mut loud, 0.01);
    assert_eq!(receive_all(&loud).first(), Some(&payload), "-57 dBFS RMS should decode");

    let mut faint = clean.clone();
    channel::scale(&mut faint, 0.002);
    let mut rx = OfdmRx::new().expect("rx alloc");
    assert!(rx.feed(&faint).expect("feed").is_empty(), "-71 dBFS RMS is below the floor");
    assert_eq!(
        rx.counters(),
        Default::default(),
        "below the floor the receiver cannot tell quiet audio from no audio"
    );
}

#[test]
fn rx_counters_track_a_clean_burst() {
    let mode = ModemMode::B170;
    let payload: Vec<u8> = (0..170u32).map(|i| (i * 17 + 9) as u8).collect();
    let burst = OfdmModem.encode_frame(mode, &payload).expect("encode");
    let pcm = splice_bursts(&[burst], &[]);

    let mut rx = OfdmRx::new().expect("rx alloc");
    assert_eq!(rx.counters(), Default::default(), "a fresh receiver starts at zero");

    let frames = rx.feed(&pcm).expect("rx feed");
    assert_eq!(frames.len(), 1);

    let c = rx.counters();
    assert_eq!(c.frames_decoded, 1, "one burst in, one frame out");
    assert_eq!(c.sync_candidates, 1, "the burst should sync exactly once");
    assert_eq!(c.preamble_failed, 0);
    assert_eq!(c.payload_failed, 0);
}

#[test]
fn rx_counters_stay_zero_on_silence() {
    // The failure signature the counters exist to distinguish: nothing is
    // reaching the modem at all, versus reaching it and decoding badly.
    let mut rx = OfdmRx::new().expect("rx alloc");
    rx.feed(&vec![0f32; 48_000 * 3]).expect("rx feed");
    assert_eq!(rx.counters(), Default::default(), "silence must not sync");
    assert_eq!(rx.last_snr_db(), -99.0, "no constellation has been demapped");
}

#[test]
fn rx_counters_survive_a_flush() {
    // flush_rx_modem rebuilds the decoder after every transmission. Field
    // diagnostics have to outlive that, or they only ever read back zero.
    let mode = ModemMode::B85;
    let payload: Vec<u8> = (0..85u32).map(|i| (i * 3 + 2) as u8).collect();
    let burst = OfdmModem.encode_frame(mode, &payload).expect("encode");

    let mut rx = OfdmRx::new().expect("rx alloc");
    rx.feed(&splice_bursts(&[burst], &[])).expect("rx feed");
    let before = rx.counters();
    assert_eq!(before.frames_decoded, 1);

    let mut fresh = OfdmRx::new().expect("rx alloc");
    fresh.adopt_counters(before);
    assert_eq!(fresh.counters(), before);
}

#[test]
fn rx_counters_report_payload_damage_not_silence() {
    // A burst whose payload symbols are corrupted but whose sync symbol is
    // intact: the modem should notice it arrived, then fail to decode it.
    // That pairing (sync_candidates > 0, frames_decoded == 0) is what an
    // impaired analog path looks like, as opposed to a dead audio route.
    let mode = ModemMode::B170;
    let payload: Vec<u8> = (0..170u32).map(|i| (i * 11 + 4) as u8).collect();
    let burst = OfdmModem.encode_frame(mode, &payload).expect("encode");
    let mut pcm = splice_bursts(&[burst], &[]);

    // Leave the sync and preamble symbols (2 * 8640) intact and blank a run
    // inside the payload symbols, the shape a dropped audio buffer takes.
    let damage_from = 24_000 + 2 * 8640 + 1_000;
    for s in &mut pcm[damage_from..damage_from + 6_000] {
        *s = 0.0;
    }

    let mut rx = OfdmRx::new().expect("rx alloc");
    let frames = rx.feed(&pcm).expect("rx feed");
    assert!(frames.is_empty(), "the payload was destroyed; nothing should decode");

    let c = rx.counters();
    assert!(c.sync_candidates > 0, "the burst still arrived: {c:?}");
    assert_eq!(c.frames_decoded, 0);
    assert!(
        c.payload_failed > 0 || c.preamble_failed > 0,
        "a damaged burst must be attributed to a stage, got {c:?}"
    );
}

#[test]
fn text_roundtrip_single_chunk() {
    let key = test_key();
    let mode = ModemMode::B170;
    let text = "hello over fm";
    let tx = frame::build_text_message(Some(key), mode, text, 0x1001, true).expect("build");
    assert_eq!(tx.frames.len(), 1);
    assert_eq!(tx.frames[0].len(), mode.frame_bytes());

    let bursts = modulate(mode, &tx.frames);
    let mut pcm = splice_bursts(&bursts, &[]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0003);
    channel::awgn(&mut pcm, 25.0, &mut rng);

    let mut rs = Reassembler::new(Some(*key));
    let event = push_frames(&mut rs, &receive_all(&pcm));
    match event {
        Some(RxEvent::Text { message_id, text: t, .. }) => {
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
    let tx = frame::build_text_message(Some(key), mode, &text, 0x1002, true).expect("build");
    assert_eq!(tx.cache.source_count, 10);
    assert_eq!(tx.frames.len(), 13);

    let bursts = modulate(mode, &tx.frames);
    let mut pcm = splice_bursts(&bursts, &[]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0004);
    channel::awgn(&mut pcm, 25.0, &mut rng);

    let mut rs = Reassembler::new(Some(*key));
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
    let tx = frame::build_text_message(Some(key), mode, &text, 0x1003, true).expect("build");
    assert_eq!(tx.frames.len(), 13);

    let bursts = modulate(mode, &tx.frames);
    // Drop two whole bursts; 11 of 13 distinct shards still exceed K = 10.
    let mut pcm = splice_bursts(&bursts, &[1, 6]);
    let mut rng = StdRng::seed_from_u64(0xAE7B0005);
    channel::awgn(&mut pcm, 25.0, &mut rng);

    let mut rs = Reassembler::new(Some(*key));
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
    let tx = frame::build_text_message(Some(key), mode, &text, 0x1004, true).expect("build");
    let k = tx.cache.source_count;
    assert_eq!(k, 10);

    // Drop 4 source chunks — beyond R = 3, so the upfront burst can't finish.
    let dropped: [u8; 4] = [0, 2, 4, 6];
    let mut rs = Reassembler::new(Some(*key));
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
    let control = frame::build_control_frame(Some(key), mode, &request, 0x2004, true).expect("control");
    assert_eq!(control.len(), mode.frame_bytes());

    // ...the sender decodes it (it's a normal encrypted frame)...
    let mut sender_rs = Reassembler::new(Some(*key));
    let parsed = match sender_rs.push_frame(&control, now).expect("control push") {
        Some(RxEvent::Control { request }) => request,
        other => panic!("expected control event, got {other:?}"),
    };
    assert_eq!(parsed, request);

    // ...and answers with cached parity from the pool (never-transmitted
    // shards, indices >= K + R).
    let repairs = frame::build_repair_frames(Some(key), &tx.cache, &parsed).expect("repairs");
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

    let mut rs = Reassembler::new(Some(*key));
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

    let mut rs = Reassembler::new(Some(*key));
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
    let tx = frame::build_text_message(Some(key), mode, &long_text(500), 0x4001, true).expect("build");

    let mut rs = Reassembler::new(Some(wrong));
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
    let tx = frame::build_text_message(Some(key), mode, "tamper me", 0x4002, true).expect("build");
    let mut bad = tx.frames[0].clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    let mut rs = Reassembler::new(Some(*key));
    assert_eq!(rs.push_frame(&bad, Instant::now()), Err(crate::AetrError::AuthFailed));
}

#[test]
fn plaintext_frame_decodes_without_a_key() {
    // A keyless (None) reassembler reads a clear frame, and a corrupted
    // trailer is rejected exactly like a bad AEAD tag.
    let mode = ModemMode::B170;
    let tx = frame::build_text_message(None, mode, "hello in the clear", 0x5001, false)
        .expect("build plaintext");
    assert_eq!(tx.frames.len(), 1);
    assert!(!tx.cache.encrypted);
    let header = Header::unpack(&tx.frames[0]).expect("header");
    assert!(header.is_plaintext());

    let mut rs = Reassembler::new(None);
    match rs.push_frame(&tx.frames[0], Instant::now()).expect("push") {
        Some(RxEvent::Text { text, encrypted, .. }) => {
            assert_eq!(text, "hello in the clear");
            assert!(!encrypted);
        }
        other => panic!("expected clear text event, got {other:?}"),
    }

    // Flip a checksum byte: the frame drops on the AuthFailed path.
    let mut bad = tx.frames[0].clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    let mut rs = Reassembler::new(None);
    assert_eq!(rs.push_frame(&bad, Instant::now()), Err(crate::AetrError::AuthFailed));
}

#[test]
fn encrypted_frame_drops_on_a_keyless_reassembler() {
    // Without a key an encrypted frame can't be opened and simply drops.
    let key = test_key();
    let mode = ModemMode::B170;
    let tx = frame::build_text_message(Some(key), mode, "sealed", 0x5002, true).expect("build");
    let mut rs = Reassembler::new(None);
    assert_eq!(rs.push_frame(&tx.frames[0], Instant::now()), Err(crate::AetrError::AuthFailed));
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


/// Generates (first run) or verifies (later runs) a golden wav, then
/// decodes the stored file and returns the resulting event.
fn golden_roundtrip(name: &str, frames: &[Vec<u8>]) -> RxEvent {
    let bursts = modulate(ModemMode::B170, frames);
    let expected = crate::wav::encode(&splice_bursts(&bursts, &[]));
    let path = testdata_path(name);
    if !std::path::Path::new(&path).exists() {
        std::fs::write(&path, &expected).expect("write golden wav");
    }
    // Compare whole file images, not just samples: that also pins the header
    // the platform apps and the analyzer parse.
    let stored = std::fs::read(&path).expect("read golden wav");
    assert!(
        stored == expected,
        "{name} drifted from the current encoder; delete testdata/{name} to regenerate"
    );
    let mut rs = Reassembler::new(Some(*test_key()));
    let pcm = crate::wav::decode(&stored).expect("golden wav must parse");
    push_frames(&mut rs, &receive_all(&pcm)).expect("golden wav must decode")
}

#[test]
fn golden_text_wav_cross_platform() {
    let tx = frame::build_text_message(Some(test_key()), ModemMode::B170, GOLDEN_TEXT, GOLDEN_TEXT_ID, true)
        .expect("build");
    assert_eq!(tx.frames.len(), 1, "golden text must stay single-chunk");
    match golden_roundtrip("golden_text.wav", &tx.frames) {
        RxEvent::Text { message_id, text, .. } => {
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


#[test]
#[ignore]
fn zzz_write_impaired_captures() {
    // Helper (run with --ignored) that writes damaged copies of the golden
    // vector so the macOS --analyze-wav report can be checked against known
    // faults instead of only against a clean file.
    let dir = std::env::var("AETR_OUT").unwrap_or_else(|_| "/tmp".into());
    let clean = crate::wav::read_wav_48k(testdata_path("golden_text.wav")).expect("read");

    let agc = channel::agc(&clean, 0.15, 0.001, 0.0001);
    crate::wav::write_wav_48k(format!("{dir}/impaired-agc.wav"), agc).expect("write");

    let mut quiet = clean.clone();
    channel::scale(&mut quiet, 0.002);
    crate::wav::write_wav_48k(format!("{dir}/impaired-quiet.wav"), quiet).expect("write");

    let mut gap = clean.clone();
    let from = 24_000 + 2 * 8640 + 1_000;
    for s in &mut gap[from..from + 6_000] {
        *s = 0.0;
    }
    crate::wav::write_wav_48k(format!("{dir}/impaired-gap.wav"), gap).expect("write");

    println!("wrote impaired captures to {dir}");
}
