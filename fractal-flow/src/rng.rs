//! Tiny deterministic RNG (SplitMix64).
//!
//! Kept in-house so a seed always produces the same scene sequence across
//! dependency upgrades.

/// SplitMix64 generator. Same seed, same sequence, forever.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Create a generator from a seed.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform f64 in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform f32 in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f64() as f32
    }

    /// Uniform index in [0, n). `n` must be > 0.
    pub fn index(&mut self, n: usize) -> usize {
        (self.f64() * n as f64) as usize % n
    }

    /// Fair coin flip.
    pub fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn ranges_stay_in_bounds() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let f = r.f64();
            assert!((0.0..1.0).contains(&f));
            let x = r.range(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&x));
            assert!(r.index(5) < 5);
        }
    }
}
