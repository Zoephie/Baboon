//! Halo CE's `seed_random`: one linear congruential seed, and the draws its
//! effects, particle systems, contrails and widgets make from it.

use crate::app::model_preview::common::random::{high24_over_2_24, high_bits, lcg_step};

/// `seed_random` and the draws made from it.
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::app::model_preview) struct CeRandom(pub(super) u32);

impl CeRandom {
    pub(super) fn seeded(seed: u32) -> Self {
        Self(seed)
    }

    /// `seed_random_real`: the next seed's high 24 bits, in [0, 1).
    pub(super) fn next(&mut self) -> f32 {
        self.0 = lcg_step(self.0);
        high24_over_2_24(self.0)
    }

    pub(super) fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    /// `seed_random_range`: an integer in [low, high).
    pub(super) fn index(&mut self, high: usize) -> usize {
        ((self.next() * high as f32) as usize).min(high.saturating_sub(1))
    }

    /// `seed_random`: the high half of the next seed.
    pub(super) fn bits(&mut self) -> u32 {
        self.0 = lcg_step(self.0);
        high_bits(self.0)
    }

    pub(super) fn direction(&mut self) -> [f32; 3] {
        let z = self.range(-1.0, 1.0);
        let (sin, cos) = self.range(0.0, std::f32::consts::TAU).sin_cos();
        let flat = (1.0 - z * z).max(0.0).sqrt();
        [flat * cos, flat * sin, z]
    }
}
