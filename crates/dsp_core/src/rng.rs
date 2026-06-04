//! Seeded, deterministic PRNG for the shared analog-noise stream and the per-hit
//! variance layer (audio-engine-spec.md §10). It never reads the wall clock or
//! any thread state — all randomness derives from `RenderRequest::seed`, which is
//! what lets identical inputs render byte-identically.
//!
//! The generator is **SplitMix64** (Steele/Lea/Flood): a single additive step
//! through a fixed odd increment followed by an avalanche mix. It is tiny,
//! branch-free, and more than good enough for noise and jitter. The magic
//! constants below are the published SplitMix64 values.

/// A SplitMix64 stream. `Clone` so a voice can fork a sub-stream from a known
/// point without disturbing the parent.
#[derive(Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// The golden-ratio odd increment (2^64 / φ).
    const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Advance the stream and return the next 64-bit value.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(Self::GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform `f32` in `[-1.0, 1.0)` — the shape the noise/variance paths want.
    /// Uses the top 24 bits so every representable mantissa step is reachable.
    #[inline]
    pub fn next_bipolar(&mut self) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32; // [0, 1)
        unit * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn bipolar_stays_in_range() {
        let mut r = SplitMix64::new(7);
        for _ in 0..10_000 {
            let v = r.next_bipolar();
            assert!((-1.0..1.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn bipolar_is_roughly_centered() {
        // Cheap sanity check that the stream isn't biased.
        let mut r = SplitMix64::new(123);
        let n = 100_000;
        let mean: f64 = (0..n).map(|_| r.next_bipolar() as f64).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.02, "mean drifted: {mean}");
    }
}
