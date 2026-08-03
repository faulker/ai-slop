//! Simulated radio channel for tests: AWGN at configurable SNR, whole-frame
//! drops (done at the test level by omitting bursts), sample-rate skew up to
//! a few hundred ppm, amplitude scaling, and DC offset.
//!
//! Plus the impairments an actual analog FM path adds, which the linear models
//! above do not cover: hard clipping, the radio's audio bandpass, FM
//! de-emphasis tilt, and a syllabic AGC. Those four are the ones that can
//! break a burst while still letting it sync, so they are what a field failure
//! most often looks like.

use rand::rngs::StdRng;
use rand::Rng;

/// The modem's occupied band, for sizing filter tests: 256 carriers at
/// 6.25 Hz spacing centred on 1500 Hz.
pub const BAND_LO_HZ: f32 = 700.0;
pub const BAND_HI_HZ: f32 = 2293.75;

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

/// Hard-clips at ±`ceiling`, the shape every overdriven stage in the chain
/// produces: a phone output at full volume, a radio's mic gain, or the FM
/// deviation limiter.
///
/// This is the impairment the modem is least able to absorb. The encoder's
/// `improve_papr` has already spent the available peak headroom, so what is
/// left is an 8.3 dB peak-to-average signal whose peaks carry real
/// information. Clipping them folds broadband intermodulation across all 256
/// carriers at once rather than damaging any one of them.
pub fn clip(samples: &mut [f32], ceiling: f32) {
    for s in samples.iter_mut() {
        *s = s.clamp(-ceiling, ceiling);
    }
}

/// One-pole high-pass, applied in place.
fn high_pass(samples: &mut [f32], cutoff_hz: f32) {
    let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
    let dt = 1.0 / 48_000.0;
    let a = rc / (rc + dt);
    let mut prev_in = 0.0f32;
    let mut prev_out = 0.0f32;
    for s in samples.iter_mut() {
        let out = a * (prev_out + *s - prev_in);
        prev_in = *s;
        prev_out = out;
        *s = out;
    }
}

/// One-pole low-pass, applied in place.
fn low_pass(samples: &mut [f32], cutoff_hz: f32) {
    let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
    let dt = 1.0 / 48_000.0;
    let a = dt / (rc + dt);
    let mut prev = 0.0f32;
    for s in samples.iter_mut() {
        prev += a * (*s - prev);
        *s = prev;
    }
}

/// Band-limits to `lo_hz`..`hi_hz` with roughly 24 dB/octave skirts, modelling
/// the audio filter in an FM radio (commonly 300-3000 Hz).
///
/// Worth testing because the modem's top carrier sits at 2293.75 Hz, close
/// enough to a 2.5-3 kHz corner that a radio rolling off early eats the upper
/// carriers. Four cascaded poles per side rather than two: at two poles the
/// skirts are so gentle that even a 300-2000 Hz "filter" leaves the top
/// carrier only ~2 dB down, which tests nothing.
pub fn bandlimit(samples: &[f32], lo_hz: f32, hi_hz: f32) -> Vec<f32> {
    let mut out = samples.to_vec();
    for _ in 0..4 {
        high_pass(&mut out, lo_hz);
        low_pass(&mut out, hi_hz);
    }
    out
}

/// Applies FM receive de-emphasis: a one-pole low-pass at the standard 750 µs
/// time constant, about -6 dB/octave above 212 Hz.
///
/// Across the modem's 700-2300 Hz band that is roughly 10 dB of tilt, which
/// matters because the decoder estimates a single scalar SNR over all 256
/// carriers rather than one per carrier. A tilted channel makes it
/// over-confident about the weak end.
pub fn deemphasis(samples: &[f32]) -> Vec<f32> {
    let mut out = samples.to_vec();
    // 750 us time constant -> corner at 1 / (2*pi*tau).
    low_pass(&mut out, 1.0 / (2.0 * std::f32::consts::PI * 750e-6));
    out
}

/// Applies a syllabic AGC: a sliding-window RMS drives a gain that chases
/// `target`, with separate attack and release rates.
///
/// This models the automatic gain control in a phone's voice-communication
/// audio path (and, loosely, the companding in a Bluetooth speech codec). It
/// is the impairment most likely to be fatal in a way that still lets the
/// burst sync, because the payload is differentially encoded across symbols:
/// the demodulator divides each carrier by its value in the previous symbol
/// (see `demod_or_erase` in the shim), which cancels a *constant* gain but
/// not one that moves between symbols 180 ms apart.
pub fn agc(samples: &[f32], target: f32, attack: f32, release: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(samples.len());
    let mut envelope = 0.0f32;
    let mut gain = 1.0f32;
    for &s in samples {
        // One-pole envelope follower on the rectified signal.
        let level = s.abs();
        let rate = if level > envelope { attack } else { release };
        envelope += rate * (level - envelope);
        let wanted = if envelope > 1e-6 { (target / envelope).clamp(0.1, 20.0) } else { 1.0 };
        gain += rate * (wanted - gain);
        out.push(s * gain);
    }
    out
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
