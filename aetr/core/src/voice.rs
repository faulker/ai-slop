//! Voice spans: codec2 at 1200 bps with 48 kHz <-> 8 kHz resampling.
//!
//! Each chunk is a self-contained span: a fresh codec2 encoder/decoder per
//! chunk so any subset of chunks plays back independently, with missing
//! spans rendered as silence at the offset implied by their chunk index.
//! Span plaintext layout: `frame_count (1 byte) || codec2 frames (6 bytes
//! each) || zero pad` to exactly the mode's chunk payload size.

use crate::AetrError;
use codec2::{Codec2, Codec2Mode};

/// codec2 MODE_1200 frame: 48 bits.
pub const CODEC2_FRAME_BYTES: usize = 6;
/// codec2 MODE_1200 frame duration at 8 kHz: 40 ms.
pub const CODEC2_FRAME_SAMPLES_8K: usize = 320;
/// Resampling ratio between the modem rate (48 kHz) and codec2 (8 kHz).
pub const RESAMPLE_FACTOR: usize = 6;

/// Number of codec2 frames that fit one chunk of the given payload size
/// (1 byte is reserved for the frame count).
pub fn frames_per_chunk(chunk_payload: usize) -> usize {
    chunk_payload.saturating_sub(1) / CODEC2_FRAME_BYTES
}

/// Windowed-sinc low-pass FIR used by both directions of the 6x resampler.
/// Cutoff ~3.4 kHz at 48 kHz (safely inside codec2's 4 kHz Nyquist),
/// Hann-windowed, 121 taps.
fn fir_taps() -> Vec<f32> {
    const TAPS: usize = 121;
    const CUTOFF: f64 = 3400.0 / 48000.0;
    let m = (TAPS - 1) as f64 / 2.0;
    let mut taps = Vec::with_capacity(TAPS);
    let mut sum = 0.0f64;
    for i in 0..TAPS {
        let x = i as f64 - m;
        let sinc = if x == 0.0 {
            2.0 * CUTOFF
        } else {
            (2.0 * std::f64::consts::PI * CUTOFF * x).sin() / (std::f64::consts::PI * x)
        };
        let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (TAPS - 1) as f64).cos();
        let t = sinc * hann;
        sum += t;
        taps.push(t);
    }
    // Normalize to unity DC gain.
    taps.into_iter().map(|t| (t / sum) as f32).collect()
}

/// Decimates 48 kHz PCM to 8 kHz: FIR low-pass, then take every 6th sample.
pub fn downsample_48k_to_8k(input: &[f32]) -> Vec<f32> {
    let taps = fir_taps();
    let half = taps.len() / 2;
    let out_len = input.len() / RESAMPLE_FACTOR;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        // Center the filter on the source sample to keep phase near zero.
        let center = n * RESAMPLE_FACTOR;
        let mut acc = 0.0f32;
        for (k, &t) in taps.iter().enumerate() {
            let idx = center as isize + k as isize - half as isize;
            if idx >= 0 && (idx as usize) < input.len() {
                acc += t * input[idx as usize];
            }
        }
        out.push(acc);
    }
    out
}

/// Interpolates 8 kHz PCM back to 48 kHz: zero-stuff by 6 and low-pass with
/// the same FIR scaled by 6 to restore amplitude.
pub fn upsample_8k_to_48k(input: &[f32]) -> Vec<f32> {
    let taps = fir_taps();
    let half = taps.len() / 2;
    let out_len = input.len() * RESAMPLE_FACTOR;
    let mut out = vec![0.0f32; out_len];
    for (n, &s) in input.iter().enumerate() {
        if s == 0.0 {
            continue;
        }
        let center = n * RESAMPLE_FACTOR;
        for (k, &t) in taps.iter().enumerate() {
            let idx = center as isize + k as isize - half as isize;
            if idx >= 0 && (idx as usize) < out_len {
                out[idx as usize] += RESAMPLE_FACTOR as f32 * t * s;
            }
        }
    }
    out
}

/// Encodes a 48 kHz mono clip into self-contained span plaintexts, each
/// exactly `chunk_payload` bytes. The clip is padded with silence to a
/// whole number of codec2 frames.
pub fn encode_clip(pcm48k: &[f32], chunk_payload: usize) -> Result<Vec<Vec<u8>>, AetrError> {
    if pcm48k.is_empty() {
        return Err(AetrError::Voice("empty clip".into()));
    }
    let per_chunk = frames_per_chunk(chunk_payload);
    if per_chunk == 0 {
        return Err(AetrError::Voice("chunk payload too small for codec2".into()));
    }
    let mut pcm8k = downsample_48k_to_8k(pcm48k);
    let frames_total = pcm8k.len().div_ceil(CODEC2_FRAME_SAMPLES_8K).max(1);
    pcm8k.resize(frames_total * CODEC2_FRAME_SAMPLES_8K, 0.0);
    // codec2 wants i16 samples.
    let speech: Vec<i16> = pcm8k
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();

    let mut spans = Vec::new();
    for chunk_frames in speech.chunks(per_chunk * CODEC2_FRAME_SAMPLES_8K) {
        // Fresh encoder per span keeps spans independently decodable.
        let mut codec = Codec2::new(Codec2Mode::MODE_1200);
        let n_frames = chunk_frames.len().div_ceil(CODEC2_FRAME_SAMPLES_8K);
        let mut span = vec![0u8; chunk_payload];
        span[0] = n_frames as u8;
        for (f, frame) in chunk_frames.chunks(CODEC2_FRAME_SAMPLES_8K).enumerate() {
            let mut padded;
            let frame = if frame.len() == CODEC2_FRAME_SAMPLES_8K {
                frame
            } else {
                padded = frame.to_vec();
                padded.resize(CODEC2_FRAME_SAMPLES_8K, 0);
                &padded
            };
            let start = 1 + f * CODEC2_FRAME_BYTES;
            codec.encode(&mut span[start..start + CODEC2_FRAME_BYTES], frame);
        }
        spans.push(span);
    }
    Ok(spans)
}

/// Decodes received spans back to 48 kHz PCM. Missing spans (None) become
/// silence at the correct offset; their indices are returned. Span length
/// for missing chunks is inferred from the received spans' payload size.
pub fn decode_clip(spans: &[Option<Vec<u8>>]) -> Result<(Vec<f32>, Vec<u32>), AetrError> {
    let payload_len = spans
        .iter()
        .flatten()
        .map(|s| s.len())
        .next()
        .ok_or_else(|| AetrError::Voice("no spans received".into()))?;
    let per_chunk = frames_per_chunk(payload_len);
    if per_chunk == 0 {
        return Err(AetrError::Voice("span too small for codec2".into()));
    }

    let mut pcm8k: Vec<f32> = Vec::new();
    let mut missing = Vec::new();
    for (idx, span) in spans.iter().enumerate() {
        match span {
            Some(bytes) => {
                if bytes.len() < 1 {
                    return Err(AetrError::Voice("span missing frame count".into()));
                }
                let n_frames = (bytes[0] as usize).min(per_chunk);
                if bytes.len() < 1 + n_frames * CODEC2_FRAME_BYTES {
                    return Err(AetrError::Voice("span truncated".into()));
                }
                // Fresh decoder per span mirrors the encoder's independence.
                let mut codec = Codec2::new(Codec2Mode::MODE_1200);
                let mut speech = vec![0i16; CODEC2_FRAME_SAMPLES_8K];
                for f in 0..n_frames {
                    let start = 1 + f * CODEC2_FRAME_BYTES;
                    codec.decode(&mut speech, &bytes[start..start + CODEC2_FRAME_BYTES]);
                    pcm8k.extend(speech.iter().map(|&s| s as f32 / i16::MAX as f32));
                }
            }
            None => {
                missing.push(idx as u32);
                // A lost span's true frame count is unknown for the final
                // chunk; assume a full span of silence.
                pcm8k.extend(std::iter::repeat(0.0f32).take(per_chunk * CODEC2_FRAME_SAMPLES_8K));
            }
        }
    }
    Ok((upsample_8k_to_48k(&pcm8k), missing))
}
