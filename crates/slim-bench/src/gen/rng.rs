//! Deterministic, seeded PRNG for dataset generation.
//!
//! SplitMix64 (Steele, Lea, Flood 2014). Chosen because it is tiny, has no
//! hidden state, and produces the same stream on every platform and every
//! rustc version -- which is what "committed, never edited" datasets need.
//! It is **not** a cryptographic PRNG and is only ever used offline in
//! `gen-datasets`.

/// SplitMix64 state.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

/// Fixed seeds, recorded in PROGRESS.md so a dataset can always be reproduced.
pub mod seed {
    /// Dataset `a` (single record).
    pub const A: u64 = 0x0000_0000_0000_00A1;
    /// Dataset `b-clean`.
    pub const B_CLEAN: u64 = 0x0000_0000_0000_0B01;
    /// Dataset `b-noisy`.
    pub const B_NOISY: u64 = 0x0000_0000_0000_0B02;
    /// Dataset `b-adversarial`.
    pub const B_ADVERSARIAL: u64 = 0x0000_0000_0000_0B03;
    /// Dataset `c` (users).
    pub const C: u64 = 0x0000_0000_0000_00C1;
    /// Dataset `d` (nested config).
    pub const D: u64 = 0x0000_0000_0000_00D1;
    /// Dataset `e` (mixed arbitrary JSON).
    pub const E: u64 = 0x0000_0000_0000_00E1;
    /// Dataset `f` (high-entropy randoms).
    pub const F: u64 = 0x0000_0000_0000_00F1;
    /// Dataset `g` (sparse, ~70% of optional fields missing).
    pub const G: u64 = 0x0000_0000_0000_00A7;
}

impl Rng {
    /// Creates a generator from a fixed seed.
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

    /// Uniform-ish value in `0..n` (`n > 0`). Modulo bias is irrelevant for
    /// synthetic telemetry and keeps the arithmetic obvious.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "range must be non-empty");
        self.next_u64() % n
    }

    /// Inclusive integer range.
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(lo <= hi);
        lo + self.below((hi - lo) as u64 + 1) as i64
    }

    /// True with probability `numerator / denominator`.
    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.below(denominator) < numerator
    }

    /// Uniform `f32` in `[0, 1)` built from 24 random mantissa bits.
    pub fn unit_f32(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as u32; // 24 bits
        f32::from_bits(0x3F80_0000 | bits) - 1.0
    }

    /// Arbitrary `f32` bit pattern (may be NaN or infinite on purpose).
    pub fn any_f32(&mut self) -> f32 {
        f32::from_bits(self.next_u64() as u32)
    }

    /// Finite `f32` in `[lo, hi]` with 0.5 granularity (telemetry speeds).
    pub fn quantized_f32(&mut self, lo: f32, hi: f32, step: f32) -> f32 {
        let steps = ((hi - lo) / step).round() as u64;
        lo + self.below(steps + 1) as f32 * step
    }

    /// Picks an element of `items`.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_is_reproducible() {
        let mut a = Rng::new(seed::B_CLEAN);
        let mut b = Rng::new(seed::B_CLEAN);
        let first: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let second: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_eq!(first, second);
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(seed::B_CLEAN);
        let mut b = Rng::new(seed::B_NOISY);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn ranges_are_respected() {
        let mut rng = Rng::new(7);
        for _ in 0..1000 {
            let v = rng.range_i64(-140, -44);
            assert!((-140..=-44).contains(&v));
            assert!(rng.below(10) < 10);
            assert!((0.0..1.0).contains(&rng.unit_f32()));
            assert!(!rng.chance(0, 5));
        }
    }

    #[test]
    fn quantized_f32_is_on_the_step_grid() {
        let mut rng = Rng::new(11);
        for _ in 0..100 {
            let v = rng.quantized_f32(0.0, 120.0, 0.5);
            assert!((0.0..=120.0).contains(&v));
            assert_eq!((v * 2.0) % 1.0, 0.0);
        }
    }
}
