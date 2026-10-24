//! Cosine color palettes: `color(t) = a + b * cos(2π(c·t + d))`.
//!
//! Four RGB vectors give smooth, endlessly cyclable gradients; shifting `t`
//! over time is what makes the colors flow.

use crate::rng::Rng;

/// Parameters of a cosine palette. Each field is an RGB triple.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub c: [f32; 3],
    pub d: [f32; 3],
}

impl Palette {
    /// Generate a vivid random palette. Brightness is centered and the swing
    /// is wide so colors stay saturated rather than muddy.
    pub fn random(rng: &mut Rng) -> Self {
        let a = [rng.range(0.45, 0.6), rng.range(0.45, 0.6), rng.range(0.45, 0.6)];
        let b = [rng.range(0.4, 0.5), rng.range(0.4, 0.5), rng.range(0.4, 0.5)];
        // Integer-ish frequencies keep the palette loopable; small detuning adds variety.
        let base = if rng.bool() { 1.0 } else { 2.0 };
        let c = [
            base * rng.range(0.8, 1.2),
            base * rng.range(0.8, 1.2),
            base * rng.range(0.8, 1.2),
        ];
        let d = [rng.range(0.0, 1.0), rng.range(0.0, 1.0), rng.range(0.0, 1.0)];
        Self { a, b, c, d }
    }

    /// Evaluate the palette at `t` (mirrors the WGSL `palette()` function).
    pub fn eval(&self, t: f32) -> [f32; 3] {
        let mut out = [0.0; 3];
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.a[i] + self.b[i] * (std::f32::consts::TAU * (self.c[i] * t + self.d[i])).cos();
        }
        out
    }

    /// Pack into four vec4s for the GPU uniform block.
    pub fn to_gpu(&self) -> [[f32; 4]; 4] {
        let p = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        [p(self.a), p(self.b), p(self.c), p(self.d)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_palettes_stay_in_displayable_range() {
        let mut rng = Rng::new(1);
        for _ in 0..500 {
            let p = Palette::random(&mut rng);
            for i in 0..100 {
                for ch in p.eval(i as f32 * 0.037) {
                    assert!((-0.06..=1.11).contains(&ch), "channel {ch} out of range");
                }
            }
        }
    }

    #[test]
    fn same_seed_same_palette() {
        assert_eq!(Palette::random(&mut Rng::new(9)), Palette::random(&mut Rng::new(9)));
    }
}
