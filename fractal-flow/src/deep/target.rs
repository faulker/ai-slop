//! Automatic zoom-target selection.
//!
//! A random zoom point is almost always flat interior or empty exterior. Instead
//! we descend one decade at a time: at each level, sample candidate points in the
//! current view, score how much structure the *next* view around each would hold
//! (spread of escape times, without being mostly interior), and step toward a
//! high scorer. The result is a point that keeps landing on spirals and minibrots.

use super::perturb_escape;
use super::reference::{precision_bits, reference_orbit, HighPoint};
use crate::quality::QualityParams;
use crate::rng::Rng;

/// View height (in the complex plane) at zoom depth 0.
pub const START_HEIGHT: f64 = 3.0;
/// Decades descended per search level.
const LEVEL_STEP: f64 = 1.0;
/// Candidate grid per axis at each level.
const GRID: usize = 7;
/// Iteration cap while searching (rendering may use more).
const SEARCH_ITER_CAP: u32 = 4_000;

/// A prepared deep-zoom destination plus the GPU reference orbit at that point.
#[derive(Clone, Debug)]
pub struct DeepTarget {
    pub center: HighPoint,
    /// Deepest zoom (decades below `START_HEIGHT`) this target supports.
    pub decades: f64,
    /// Reference orbit at `center`, as f32 pairs for the GPU storage buffer.
    pub orbit: Vec<[f32; 2]>,
}

/// Search for a structure-rich target `decades` deep. Deterministic for a given seed.
/// This is CPU heavy (seconds at extreme depths), so call it off the render thread.
pub fn find_target(seed: u64, decades: f64, quality: &QualityParams) -> DeepTarget {
    let mut rng = Rng::new(seed);
    let final_prec = precision_bits(decades + 4.0);
    // Start somewhere along the boundary rather than always at the same spot.
    const STARTS: [[f64; 2]; 4] = [[-0.75, 0.0], [-0.75, 0.1], [-1.25, 0.0], [0.28, 0.0]];
    let start = STARTS[rng.index(STARTS.len())];
    let mut center = HighPoint::from_f64(start[0], start[1], final_prec);

    let mut depth = 0.0;
    while depth < decades {
        let h = START_HEIGHT * 10f64.powf(-depth);
        let iters = quality.iterations_at_depth(depth + LEVEL_STEP).min(SEARCH_ITER_CAP);
        let orbit = reference_orbit(&center, iters, precision_bits(depth + 4.0).min(final_prec));
        if let Some(offset) = pick_candidate(&orbit, h, iters, &mut rng) {
            center = center.offset(offset[0], offset[1], final_prec);
        }
        depth += LEVEL_STEP;
    }

    let render_iters = quality.max_iterations.max(quality.iterations_at_depth(decades));
    let orbit = reference_orbit(&center, render_iters, final_prec).iter().map(|z| [z[0] as f32, z[1] as f32]).collect();
    DeepTarget { center, decades, orbit }
}

/// Score candidates in a view of height `h` around the orbit's center and pick one
/// of the best (randomly among the top 3 for variety). `None` if all are boring.
fn pick_candidate(orbit: &[[f64; 2]], h: f64, iters: u32, rng: &mut Rng) -> Option<[f64; 2]> {
    let next_h = h * 10f64.powf(-LEVEL_STEP);
    let mut scored: Vec<(f64, [f64; 2])> = Vec::with_capacity(GRID * GRID);
    for gy in 0..GRID {
        for gx in 0..GRID {
            let jitter = [rng.range(-0.5, 0.5) as f64, rng.range(-0.5, 0.5) as f64];
            let o = [
                ((gx as f64 + 0.5 + jitter[0]) / GRID as f64 - 0.5) * 0.8 * h,
                ((gy as f64 + 0.5 + jitter[1]) / GRID as f64 - 0.5) * 0.8 * h,
            ];
            if let Some(score) = score_view(orbit, o, next_h, iters) {
                scored.push((score, o));
            }
        }
    }
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let top = scored.len().min(3);
    Some(scored[rng.index(top)].1)
}

/// Structure score of a view of height `h` centered at offset `o`: the spread of
/// log escape times over a 3×3 sample grid, nudged toward higher iteration counts
/// (closer to the set = finer detail). `None` if mostly interior or barely iterated.
fn score_view(orbit: &[[f64; 2]], o: [f64; 2], h: f64, iters: u32) -> Option<f64> {
    let mut logs = Vec::with_capacity(9);
    let mut interior = 0;
    for sy in -1..=1 {
        for sx in -1..=1 {
            let dc = [o[0] + sx as f64 * h * 0.4, o[1] + sy as f64 * h * 0.4];
            match perturb_escape(orbit, dc, iters) {
                Some(n) => logs.push(n.max(1.0).ln()),
                None => interior += 1,
            }
        }
    }
    if interior >= 4 || logs.len() < 3 {
        return None;
    }
    let mean = logs.iter().sum::<f64>() / logs.len() as f64;
    if mean < 2.0 {
        return None;
    }
    let var = logs.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / logs.len() as f64;
    Some(var.sqrt() + 0.1 * mean + 0.05 * interior as f64)
}

#[cfg(test)]
mod tests {
    use super::super::direct_escape;
    use super::*;

    fn quality() -> QualityParams {
        crate::quality::QualityPreset::Low.params().unwrap()
    }

    /// True if `c` is in the main cardioid or period-2 bulb (flat interior).
    fn in_main_body(c: [f64; 2]) -> bool {
        let (x, y) = (c[0], c[1]);
        let q = (x - 0.25).powi(2) + y * y;
        q * (q + (x - 0.25)) <= 0.25 * y * y || (x + 1.0).powi(2) + y * y <= 1.0 / 16.0
    }

    #[test]
    fn target_lands_on_boundary_structure() {
        for seed in 0..4 {
            let t = find_target(seed, 8.0, &quality());
            let c = t.center.to_f64();
            assert!(!in_main_body(c), "seed {seed}: target {c:?} is in the main cardioid/bulb");
            // Near the boundary: escapes late, or is interior right next to escaping points.
            let n = direct_escape(c, 20_000);
            assert!(n.is_none_or(|n| n > 30.0), "seed {seed}: target escapes too fast ({n:?})");
            let h = START_HEIGHT * 1e-8;
            let neighbors: Vec<_> = (0..8)
                .map(|k| {
                    let a = k as f64 * std::f64::consts::TAU / 8.0;
                    direct_escape([c[0] + h * a.cos(), c[1] + h * a.sin()], 20_000)
                })
                .collect();
            assert!(neighbors.iter().any(|n| n.is_some()), "seed {seed}: final view is all interior");
        }
    }

    #[test]
    fn same_seed_same_target() {
        let a = find_target(77, 6.0, &quality());
        let b = find_target(77, 6.0, &quality());
        assert_eq!(a.center, b.center);
        assert_eq!(a.orbit, b.orbit);
    }

    #[test]
    fn deep_targets_keep_precision() {
        let t = find_target(5, 40.0, &quality());
        assert_eq!(t.decades, 40.0);
        assert!(t.orbit.len() >= 2);
        assert!(t.center.re.precision() >= precision_bits(40.0));
    }
}
