//! Simulated radio channel for tests: AWGN at configurable SNR, whole-frame
//! drops (done at the test level by omitting bursts), sample-rate skew up to
//! a few hundred ppm, amplitude scaling, and DC offset.

use rand::rngs::StdRng;
use rand::Rng;

/// Adds white Gaussian noise at the given SNR relative to the measured
/// power of the non-silent part of the signal.
pub fn awgn(samples: &mut [f32], snr_db: f32, rng: &mut StdRng) {
    let power: f32 = samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32;
    if power <= 0.0 {
        return;
    }
    let noise_power = power / 10f32.powf(snr_db / 10.0);
    let sigma = noise_power.sqrt();
    // Box-Muller transform; avoids pulling in rand_distr.
    let mut i = 0;
    while i < samples.len() {
        let u1: f32 = rng.gen_range(f32::EPSILON..1.0);
        let u2: f32 = rng.gen_range(0.0..1.0);
        let mag = (-2.0 * u1.ln()).sqrt();
        let ang = 2.0 * std::f32::consts::PI * u2;
        samples[i] += sigma * mag * ang.cos();
        i += 1;
        if i < samples.len() {
            samples[i] += sigma * mag * ang.sin();
            i += 1;
        }
    }
}

/// Resamples with a constant clock skew in parts-per-million using linear
/// interpolation (models mismatched sound card clocks, up to ~±200 ppm).
pub fn resample_skew(samples: &[f32], ppm: f32) -> Vec<f32> {
    let step = 1.0f64 + ppm as f64 * 1e-6;
    let mut out = Vec::with_capacity(samples.len());
    let mut pos = 0.0f64;
    loop {
        let i = pos as usize;
        if i + 1 >= samples.len() {
            break;
        }
        let frac = (pos - i as f64) as f32;
        out.push(samples[i] * (1.0 - frac) + samples[i + 1] * frac);
        pos += step;
    }
    out
}

/// Scales amplitude (models radio volume/AGC differences).
pub fn scale(samples: &mut [f32], factor: f32) {
    for s in samples.iter_mut() {
        *s *= factor;
    }
}

/// Adds a constant DC offset (models cheap sound card bias).
pub fn dc_offset(samples: &mut [f32], offset: f32) {
    for s in samples.iter_mut() {
        *s += offset;
    }
}

/// Convenience: applies skew, scaling, DC offset, and AWGN in one pass, the
/// combination a real FM radio path would show.
pub fn impaired(samples: &[f32], snr_db: f32, ppm: f32, gain: f32, dc: f32, rng: &mut StdRng) -> Vec<f32> {
    let mut out = resample_skew(samples, ppm);
    scale(&mut out, gain);
    dc_offset(&mut out, dc);
    awgn(&mut out, snr_db, rng);
    out
}
