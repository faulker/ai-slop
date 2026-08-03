//! Canonical 48 kHz mono 16-bit PCM WAV read/write.
//!
//! One implementation shared by three callers that must agree byte for byte:
//! the golden-vector tests, the field capture the platform apps write, and the
//! offline analyzer that decodes those captures. 16-bit is the storage format
//! (not the modem's f32) because that is what the golden vectors already use
//! and what every audio tool opens without argument.

use crate::AetrError;

/// The only sample rate anything in aetr speaks.
pub const SAMPLE_RATE: u32 = 48_000;
/// Bytes of RIFF/fmt/data header ahead of the samples.
const HEADER_BYTES: usize = 44;

/// Converts f32 PCM to 16-bit with clamping.
fn f32_to_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect()
}

/// Converts 16-bit samples back to f32.
fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 32768.0).collect()
}

/// Serializes samples into a complete 48 kHz mono 16-bit WAV file image.
pub fn encode(pcm: &[f32]) -> Vec<u8> {
    let samples = f32_to_i16(pcm);
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(HEADER_BYTES + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in &samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Parses a 48 kHz mono 16-bit PCM WAV image, walking the RIFF chunk list so
/// files carrying extra metadata chunks still load. Rejects any other format
/// rather than silently misreading it, since a capture at the wrong rate would
/// look like a broken radio path instead of a broken capture.
pub fn decode(bytes: &[u8]) -> Result<Vec<f32>, AetrError> {
    if bytes.len() < HEADER_BYTES || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AetrError::Malformed("not a RIFF/WAVE file".into()));
    }
    let mut pos = 12;
    let mut data: Option<&[u8]> = None;
    let mut saw_fmt = false;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let end = pos + 8 + len;
        if end > bytes.len() {
            return Err(AetrError::Malformed(format!("chunk at {pos} runs past end of file")));
        }
        let body = &bytes[pos + 8..end];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(AetrError::Malformed("fmt chunk is too short".into()));
                }
                let format = u16::from_le_bytes(body[0..2].try_into().unwrap());
                let channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
                if format != 1 || channels != 1 || rate != SAMPLE_RATE || bits != 16 {
                    return Err(AetrError::Malformed(format!(
                        "expected 48 kHz mono 16-bit PCM, got format {format}, \
                         {channels} channel(s), {rate} Hz, {bits}-bit"
                    )));
                }
                saw_fmt = true;
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos = end + (len & 1); // chunks are word-aligned
    }
    if !saw_fmt {
        return Err(AetrError::Malformed("no fmt chunk".into()));
    }
    let data = data.ok_or_else(|| AetrError::Malformed("no data chunk".into()))?;
    let samples: Vec<i16> =
        data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    Ok(i16_to_f32(&samples))
}

/// Writes samples to a 48 kHz mono 16-bit WAV file.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn write_wav_48k(path: String, pcm: Vec<f32>) -> Result<(), AetrError> {
    std::fs::write(&path, encode(&pcm)).map_err(|e| AetrError::Io(format!("{path}: {e}")))
}

/// Reads a 48 kHz mono 16-bit WAV file.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn read_wav_48k(path: String) -> Result<Vec<f32>, AetrError> {
    let bytes = std::fs::read(&path).map_err(|e| AetrError::Io(format!("{path}: {e}")))?;
    decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_the_16_bit_format() {
        let pcm: Vec<f32> =
            (0..4_800).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let decoded = decode(&encode(&pcm)).expect("decode");
        assert_eq!(decoded.len(), pcm.len());
        for (a, b) in pcm.iter().zip(&decoded) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b} exceeds 16-bit quantization");
        }
    }

    #[test]
    fn clamps_instead_of_wrapping() {
        // Wrapping would turn a hot sample into full-scale of the opposite
        // sign, which reads as impulsive noise rather than the clipping it is.
        let decoded = decode(&encode(&[2.0, -2.0])).expect("decode");
        assert!(decoded[0] > 0.99 && decoded[1] < -0.99, "got {decoded:?}");
    }

    #[test]
    fn preserves_length_and_header_size() {
        let image = encode(&vec![0.0; 100]);
        assert_eq!(image.len(), HEADER_BYTES + 200);
        assert_eq!(&image[0..4], b"RIFF");
    }

    #[test]
    fn rejects_a_capture_at_the_wrong_rate() {
        let mut image = encode(&vec![0.0; 10]);
        image[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        let err = decode(&image).expect_err("44.1 kHz must be rejected");
        assert!(format!("{err}").contains("44100"), "unhelpful error: {err}");
    }

    #[test]
    fn rejects_non_wav_input() {
        assert!(decode(b"not a wav file at all, not even close").is_err());
    }

    #[test]
    fn skips_unknown_chunks() {
        // Recorders routinely emit LIST/INFO chunks between fmt and data.
        let base = encode(&vec![0.25; 8]);
        let mut image = base[..36].to_vec();
        image.extend_from_slice(b"LIST");
        image.extend_from_slice(&4u32.to_le_bytes());
        image.extend_from_slice(b"INFO");
        image.extend_from_slice(&base[36..]);
        let riff_len = (image.len() - 8) as u32;
        image[4..8].copy_from_slice(&riff_len.to_le_bytes());
        assert_eq!(decode(&image).expect("decode").len(), 8);
    }
}
