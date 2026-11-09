//! Seed allocation for particle properties, with Reach's seed tables that Halo 4 shares.

use super::*;

/// `allocate_random_seed`'s two counters, handing seeds out in property order,
/// and what the game's state list leaves of the inputs it does not seed.
#[derive(Default)]
pub(in crate::app::model_preview) struct SeedAllocator {
    particle: usize,
    system: usize,
    /// Halo 3 and ODST: `particle rotation` reads the 0 their state list writes.
    rotation_unread: bool,
    /// Reach's: seeds go by which states are taken, not by counting.
    pub(super) reach: Option<ReachSeeds>,
}

/// Reach tool's `allocate_random_seed` (0x14106CFA0): the highest seed not
/// yet taken, and once none is left, the next of a table of reuses.
#[derive(Debug, Clone, Copy)]
pub(super) struct ReachSeeds {
    /// The states taken, by [`ParticleInput::state_bit`].
    pub(super) used: u32,
    /// `dword_1428F5DE4`, shared by both tables. The shipped tool starts it at
    /// 4 and never resets it, so it counts every run-out of every tag a run
    /// builds; the preview starts it at 4 for each allocation instead.
    reuse: usize,
    /// A movement controller's (`fully_index_parameters_and_postprocess`, tool
    /// 0x140DD92D0): each parameter's own states are marked taken just before
    /// its randoms are allocated, rather than all of them first.
    pub(super) marks_each: bool,
}

/// `particle random`'s seeds, highest state first: 25 to 22, then 7 to 4.
const REACH_PARTICLE_SEEDS: [ParticleInput; 8] = {
    use ParticleInput::Seed;
    [Seed(7), Seed(6), Seed(5), Seed(4), Seed(3), Seed(2), Seed(1), Seed(0)]
};

/// The particle reuse table (tool 0x141FFF3C0): states 22, 7, 23, 6, 24, 5, 25, 4.
const REACH_PARTICLE_REUSE: [ParticleInput; 8] = {
    use ParticleInput::Seed;
    [Seed(4), Seed(3), Seed(5), Seed(2), Seed(6), Seed(1), Seed(7), Seed(0)]
};

/// `system random`'s seeds, highest state first, which is also the system
/// reuse table (tool 0x141FFF3E0): states 27, 26, 9, 8.
const REACH_SYSTEM_SEEDS: [ParticleInput; 4] = {
    use ParticleInput::SystemSeed;
    [SystemSeed(3), SystemSeed(2), SystemSeed(1), SystemSeed(0)]
};

/// What tool's postprocesses keep of a mask of used states before
/// allocating: all but the two pseudo states, `particle random` and `system
/// random`.
pub(super) const REACH_ALLOCATED_STATES: u32 = 0x0FFF_FFF3;

impl ReachSeeds {
    fn take(&mut self, seeds: &[ParticleInput], reuse: &[ParticleInput]) -> ParticleInput {
        let free = seeds.iter().copied().find(|seed| self.used & seed.state_bit() == 0);
        let seed = free.unwrap_or_else(|| {
            let seed = reuse[self.reuse % reuse.len()];
            self.reuse += 1;
            seed
        });
        self.used |= seed.state_bit();
        seed
    }
}

impl SeedAllocator {
    /// For a Halo 3 or ODST tag's properties.
    pub(super) fn halo3() -> Self {
        Self { rotation_unread: true, ..Self::default() }
    }

    /// For a Reach tag's properties, with `used` the states its definition
    /// reads already. Reach writes 0 for `particle rotation`, as Halo 3 does.
    pub(super) fn reach(used: u32) -> Self {
        let seeds = ReachSeeds { used, reuse: 4, marks_each: false };
        Self { rotation_unread: true, reach: Some(seeds), ..Self::default() }
    }

    /// For one of a Reach movement's controllers: a mask of its own.
    pub(super) fn reach_controller() -> Self {
        let mut seeds = Self::reach(0);
        seeds.reach.iter_mut().for_each(|reach| reach.marks_each = true);
        seeds
    }

    /// The states Reach's allocations have taken so far; none for Halo 3's.
    pub(super) fn used(&self) -> u32 {
        self.reach.map_or(0, |reach| reach.used)
    }

    pub(super) fn allocate(&mut self, input: &mut ParticleInput) {
        match (*input, self.reach.as_mut()) {
            (ParticleInput::Rotation, _) if self.rotation_unread => *input = ParticleInput::Other,
            (ParticleInput::Random, Some(reach)) => *input = reach.take(&REACH_PARTICLE_SEEDS, &REACH_PARTICLE_REUSE),
            (ParticleInput::SystemRandom, Some(reach)) => *input = reach.take(&REACH_SYSTEM_SEEDS, &REACH_SYSTEM_SEEDS),
            (ParticleInput::Random, None) => {
                *input = ParticleInput::Seed(self.particle % SEEDS);
                self.particle += 1;
            }
            (ParticleInput::SystemRandom, None) => {
                *input = ParticleInput::SystemSeed(self.system % SYSTEM_SEEDS);
                self.system += 1;
            }
            _ => {}
        }
    }
}
