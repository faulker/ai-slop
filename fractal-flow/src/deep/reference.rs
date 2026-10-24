//! Arbitrary-precision reference orbit for perturbation rendering.

use dashu_float::FBig;

/// A complex number at arbitrary precision.
#[derive(Clone, Debug, PartialEq)]
pub struct HighPoint {
    pub re: FBig,
    pub im: FBig,
}

/// Bits of precision needed to address points at a zoom depth of `decades`
/// (log2(10) ≈ 3.33 bits per decade, plus headroom).
pub fn precision_bits(decades: f64) -> usize {
    (decades.max(0.0) * 3.33).ceil() as usize + 64
}

/// Convert an f64 to an `FBig` with `prec` bits.
pub fn big(x: f64, prec: usize) -> FBig {
    FBig::try_from(x).expect("finite f64").with_precision(prec).value()
}

impl HighPoint {
    /// From f64 coordinates.
    pub fn from_f64(re: f64, im: f64, prec: usize) -> Self {
        Self { re: big(re, prec), im: big(im, prec) }
    }

    /// Add an f64 offset (exact up to `prec` bits).
    pub fn offset(&self, dre: f64, dim: f64, prec: usize) -> Self {
        Self {
            re: (self.re.clone().with_precision(prec).value() + big(dre, prec)).with_precision(prec).value(),
            im: (self.im.clone().with_precision(prec).value() + big(dim, prec)).with_precision(prec).value(),
        }
    }

    /// Nearest f64 approximation (for display only).
    pub fn to_f64(&self) -> [f64; 2] {
        [self.re.to_f64().value(), self.im.to_f64().value()]
    }
}

/// Orbit of `z → z² + c` from `z0 = 0`, rounded to f64 per step, until it escapes
/// (|z|² > 4) or `max_iter` steps. `orbit[0] = 0`, `orbit[1] = c`.
pub fn reference_orbit(c: &HighPoint, max_iter: u32, prec: usize) -> Vec<[f64; 2]> {
    let cr = c.re.clone().with_precision(prec).value();
    let ci = c.im.clone().with_precision(prec).value();
    let mut zr = big(0.0, prec);
    let mut zi = big(0.0, prec);
    let mut orbit = Vec::with_capacity(max_iter as usize + 1);
    orbit.push([0.0, 0.0]);
    for _ in 0..max_iter {
        let zr2 = &zr * &zr;
        let zi2 = &zi * &zi;
        let zri = &zr * &zi;
        zr = zr2 - zi2 + &cr;
        zi = &zri + &zri + &ci;
        let f = [zr.to_f64().value(), zi.to_f64().value()];
        orbit.push(f);
        if f[0] * f[0] + f[1] * f[1] > 4.0 {
            break;
        }
    }
    orbit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_f64_at_shallow_depth() {
        let c = [-0.1011, 0.9563];
        let orbit = reference_orbit(&HighPoint::from_f64(c[0], c[1], 128), 200, 128);
        let (mut zr, mut zi) = (0.0f64, 0.0f64);
        for (n, z) in orbit.iter().enumerate().take(30) {
            assert!((z[0] - zr).abs() < 1e-9 && (z[1] - zi).abs() < 1e-9, "step {n} diverged");
            let t = zr * zr - zi * zi + c[0];
            zi = 2.0 * zr * zi + c[1];
            zr = t;
        }
    }

    #[test]
    fn stops_on_escape_and_runs_full_length_inside() {
        assert!(reference_orbit(&HighPoint::from_f64(1.0, 1.0, 64), 1000, 64).len() < 5);
        assert_eq!(reference_orbit(&HighPoint::from_f64(-0.1, 0.1, 64), 1000, 64).len(), 1001);
    }

    #[test]
    fn precision_grows_with_depth() {
        assert!(precision_bits(30.0) > precision_bits(10.0));
        assert!(precision_bits(100.0) >= 333 + 64);
    }

    #[test]
    fn offsets_below_f64_resolution_are_kept() {
        let prec = precision_bits(40.0);
        let p = HighPoint::from_f64(-0.75, 0.1, prec).offset(1e-35, 0.0, prec);
        let back = p.offset(-1e-35, 0.0, prec);
        assert_ne!(p.re, HighPoint::from_f64(-0.75, 0.1, prec).re, "1e-35 offset was lost");
        assert_eq!(back.re, HighPoint::from_f64(-0.75, 0.1, prec).re);
    }
}
