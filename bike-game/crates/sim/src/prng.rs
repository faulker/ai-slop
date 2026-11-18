//! Tiny deterministic PRNG for seed-based course generation.
//!
//! Integer math only. Same seed bytes always produce the same sequence
//! across runs and machines.

/// SplitMix64-style mixer followed by xorshift64*.
#[derive(Clone, Debug)]
pub struct Prng {
    state: u64,
}

impl Prng {
    /// Creates a PRNG from an arbitrary seed string (UTF-8 bytes mixed in).
    pub fn from_seed_str(seed: &str) -> Self {
        let mut h: u64 = 0x9E37_79B9_7F4A_7C15;
        for &b in seed.as_bytes() {
            h = h.wrapping_add(u64::from(b));
            h = h.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            h ^= h >> 33;
        }
        // Avoid the all-zero state (xorshift degenerates).
        if h == 0 {
            h = 0xDEAD_BEEF_CAFE_BABE;
        }
        Self { state: h }
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform float in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as u32; // 24 bits of mantissa
        bits as f32 / ((1u32 << 24) as f32)
    }

    /// Uniform float in [lo, hi).
    pub fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    /// Uniform integer in [lo, hi] inclusive.
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo) as u64 + 1;
        lo + (self.next_u64() % span) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Prng::from_seed_str("neon");
        let mut b = Prng::from_seed_str("neon");
        for _ in 0..32 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn empty_seed_is_stable() {
        let mut a = Prng::from_seed_str("");
        let mut b = Prng::from_seed_str("");
        assert_eq!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Prng::from_seed_str("alpha");
        let mut b = Prng::from_seed_str("beta");
        assert_ne!(a.next_u64(), b.next_u64());
    }
}
