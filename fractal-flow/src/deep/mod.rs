//! Deep Mandelbrot zooms via perturbation.
//!
//! f32 runs out of precision around 1e-5 zoom. Instead, one reference orbit is
//! computed at arbitrary precision on the CPU (`reference`), and every pixel only
//! tracks its small difference from that orbit. `target` picks where to zoom.
//! `perturb_escape` is the CPU twin of `shaders/mandel_deep.wgsl`, used by the
//! target search and by tests that prove the math against direct iteration.

pub mod reference;
pub mod target;

pub use target::{find_target, DeepTarget};

/// Escape radius squared. Large so the smooth iteration count is accurate.
pub const BAILOUT2: f64 = 1.0e6;

/// Smooth (continuous) iteration count for a point that escaped at iteration `i`
/// with squared magnitude `r2`.
pub fn smooth_iter(i: u32, r2: f64) -> f64 {
    i as f64 + 1.0 - (0.5 * r2.ln()).ln() / std::f64::consts::LN_2
}

/// Escape time of `c = C + dc` by perturbation against the reference orbit of `C`
/// (`orbit[0] = 0`, `orbit[n+1] = orbit[n]² + C`). Uses rebasing so any reference,
/// even one that escapes early, gives correct results. `None` = did not escape.
pub fn perturb_escape(orbit: &[[f64; 2]], dc: [f64; 2], max_iter: u32) -> Option<f64> {
    debug_assert!(orbit.len() >= 2, "reference orbit needs at least z0 and z1");
    let mut dz = dc;
    let mut m = 1usize;
    for i in 1..max_iter {
        let zr = orbit[m][0] + dz[0];
        let zi = orbit[m][1] + dz[1];
        let r2 = zr * zr + zi * zi;
        if r2 > BAILOUT2 {
            return Some(smooth_iter(i, r2));
        }
        // Rebase onto the start of the orbit when the pixel orbit gets closer to
        // zero than its delta (or the reference ran out); avoids glitches.
        if r2 < dz[0] * dz[0] + dz[1] * dz[1] || m + 1 >= orbit.len() {
            dz = [zr, zi];
            m = 0;
        }
        let [ar, ai] = orbit[m];
        // dz' = 2·Z·dz + dz² + dc
        let nr = 2.0 * (ar * dz[0] - ai * dz[1]) + (dz[0] * dz[0] - dz[1] * dz[1]) + dc[0];
        let ni = 2.0 * (ar * dz[1] + ai * dz[0]) + 2.0 * dz[0] * dz[1] + dc[1];
        dz = [nr, ni];
        m += 1;
    }
    None
}

/// Plain f64 escape time of `c`, for checking `perturb_escape` at shallow zooms.
pub fn direct_escape(c: [f64; 2], max_iter: u32) -> Option<f64> {
    let (mut zr, mut zi) = (0.0f64, 0.0f64);
    for i in 0..max_iter {
        let r2 = zr * zr + zi * zi;
        if r2 > BAILOUT2 {
            return Some(smooth_iter(i, r2));
        }
        let t = zr * zr - zi * zi + c[0];
        zi = 2.0 * zr * zi + c[1];
        zr = t;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::reference::{reference_orbit, HighPoint};
    use super::*;

    /// Compare perturbation against direct f64 on a pixel grid around `center`.
    fn agreement(center: [f64; 2], half: f64, max_iter: u32) -> f64 {
        let hp = HighPoint::from_f64(center[0], center[1], 128);
        let orbit = reference_orbit(&hp, max_iter, 128);
        let n = 24;
        let mut agree = 0;
        for y in 0..n {
            for x in 0..n {
                let dc = [half * (x as f64 / (n - 1) as f64 * 2.0 - 1.0), half * (y as f64 / (n - 1) as f64 * 2.0 - 1.0)];
                let p = perturb_escape(&orbit, dc, max_iter);
                let d = direct_escape([center[0] + dc[0], center[1] + dc[1]], max_iter);
                let ok = match (p, d) {
                    (None, None) => true,
                    (Some(a), Some(b)) => (a - b).abs() < 1.0,
                    _ => false,
                };
                agree += ok as usize;
            }
        }
        agree as f64 / (n * n) as f64
    }

    #[test]
    fn perturbation_matches_direct_at_1e6_zoom() {
        // Seahorse valley, a boundary region with lots of structure.
        let frac = agreement([-0.743_643_887_037_151, 0.131_825_904_205_330], 1.5e-6, 4000);
        assert!(frac >= 0.97, "only {:.1}% of pixels agree", frac * 100.0);
    }

    #[test]
    fn perturbation_matches_direct_with_escaping_reference() {
        // Reference outside the set escapes quickly; rebasing must still give correct counts.
        let frac = agreement([-0.75, 0.2], 0.05, 1000);
        assert!(frac >= 0.97, "only {:.1}% of pixels agree", frac * 100.0);
    }

    #[test]
    fn interior_and_exterior_basics() {
        assert!(direct_escape([0.0, 0.0], 500).is_none());
        assert!(direct_escape([2.0, 2.0], 500).unwrap() < 3.0);
    }
}
