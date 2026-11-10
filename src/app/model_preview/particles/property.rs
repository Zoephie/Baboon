//! Particle properties: the state inputs, a property function with its inputs and seeds, and level-of-detail amounts.

use super::rules::{SEEDS, SYSTEM_SEEDS};
use super::seeds::SeedAllocator;
use super::shader::{read_ends, slerped};
use super::tag_function_at;
use blam_tags::{TagFunction, TagStruct};
use std::sync::Arc;

/// `game_state_type_enum`, by option name. Those with a `{` carry another
/// name after it, which a tag's own schema may spell the first way only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParticleInput {
    Age,
    SystemAge,
    Random,
    SystemRandom,
    Seed(usize),
    SystemSeed(usize),
    EmissionTime,
    Lod,
    GameTime,
    EffectA,
    EffectB,
    /// The particle's own rotation, as Halo 2 reads it. Halo 3 and ODST's
    /// `update_states_internal` write 0 here instead (MCC halo3.dll
    /// 0x180221B18, ODST 0x1406D46B0), which [`SeedAllocator::counting`] keeps.
    Rotation,
    LocationSeed,
    /// Reach's `particle velocity`, 20: `get_state_value` reads
    /// `length(m_velocity) * 0.1`, the speed in the particle's own frame. It
    /// is an evolving state only, so what is read at a spawn reads 0.
    Velocity,
    /// What reads 0: an unknown name, the invalid state, or one no game writes
    /// (`distance from emitter`, 17: neither state list sets it, and the card's
    /// upload mask 0xFE117B02 leaves it out).
    Other,
}

impl ParticleInput {
    pub(super) fn named(name: &str) -> Self {
        match name.split('{').next().unwrap_or_default() {
            "particle age" => Self::Age,
            "system age" | "emitter age" => Self::SystemAge,
            "particle random" => Self::Random,
            "system random" | "emitter random" => Self::SystemRandom,
            "particle correlation 1" | "particle random 1" => Self::Seed(0),
            "particle correlation 2" | "particle random 2" => Self::Seed(1),
            "particle correlation 3" | "particle random 3" => Self::Seed(2),
            "particle correlation 4" | "particle random 4" => Self::Seed(3),
            "system correlation 1" | "emitter correlation 1" | "emitter random 1" => {
                Self::SystemSeed(0)
            }
            "system correlation 2" | "emitter correlation 2" | "emitter random 2" => {
                Self::SystemSeed(1)
            }
            "particle emission time" | "emitter time" | "particle emit time" => Self::EmissionTime,
            "location lod" | "system lod" => Self::Lod,
            "game time" => Self::GameTime,
            "effect a scale" => Self::EffectA,
            "effect b scale" => Self::EffectB,
            "particle rotation" | "physics rotation" => Self::Rotation,
            "location random" => Self::LocationSeed,
            "particle velocity" => Self::Velocity,
            // Reach's `update_states_internal` (the 360's 0x82ED14A0, MCC
            // sub_1405CD4D0) never writes 17, and 18 and 19 only for a boat hull
            // or jetwash emitter, which the preview does not run; the card's
            // upload mask 0xFC217B02 leaves all three out, so the game reads
            // whatever the slot held. 0 is the preview's choice.
            "distance from emitter" | "particle simulation a" | "particle simulation b" => {
                Self::Other
            }
            _ => Self::Other,
        }
    }

    pub(super) fn of(self, state: &ParticleState) -> f32 {
        match self {
            Self::Age => state.age,
            Self::SystemAge => state.system_age,
            Self::Seed(seed) => state.seeds[seed % SEEDS],
            Self::SystemSeed(seed) => state.system_seeds[seed % SYSTEM_SEEDS],
            Self::EmissionTime => state.emission_time,
            Self::Lod => state.lod,
            Self::GameTime => state.game_time,
            Self::EffectA => state.effect_a,
            Self::EffectB => state.effect_b,
            Self::Rotation => state.rotation,
            Self::LocationSeed => state.location_seed,
            Self::Velocity => state.velocity,
            Self::Random | Self::SystemRandom | Self::Other => 0.0,
        }
    }

    /// `1 <<` its index in Reach's `particle_state_list.hlsl`, as tool's masks
    /// of used states hold it; none for what the preview reads as 0.
    pub(super) fn state_bit(self) -> u32 {
        let index = match self {
            Self::Age => 0,
            Self::SystemAge => 1,
            Self::Random => 2,
            Self::SystemRandom => 3,
            // Seeds 1 to 4 are states 4 to 7, and 5 to 8 are 22 to 25.
            Self::Seed(seed) if seed < 4 => 4 + seed,
            Self::Seed(seed) => 22 + seed - 4,
            // System seeds 1 and 2 are states 8 and 9, and 3 and 4 are 26 and 27.
            Self::SystemSeed(seed) if seed < 2 => 8 + seed,
            Self::SystemSeed(seed) => 26 + seed - 2,
            Self::EmissionTime => 10,
            Self::Lod => 11,
            Self::GameTime => 12,
            Self::EffectA => 13,
            Self::EffectB => 14,
            Self::Rotation => 15,
            Self::LocationSeed => 16,
            Self::Velocity => 20,
            Self::Other => return 0,
        };
        1 << index
    }
}

/// `c_particle_state_list`.
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::app::model_preview) struct ParticleState {
    pub(super) age: f32,
    pub(super) system_age: f32,
    pub(super) seeds: [f32; SEEDS],
    pub(super) system_seeds: [f32; SYSTEM_SEEDS],
    pub(super) emission_time: f32,
    pub(super) game_time: f32,
    pub(super) effect_a: f32,
    pub(super) effect_b: f32,
    pub(super) rotation: f32,
    pub(super) location_seed: f32,
    pub(super) lod: f32,
    /// Reach's `particle velocity`.
    pub(super) velocity: f32,
}

/// `c_particle_system_definition::calc_lod_amount`: distances in, out, and how
/// far each fades over. None for `lod always 1.0`.
pub(in crate::app::model_preview) fn lod_amount(lod: Option<[f32; 4]>, distance: f32) -> f32 {
    let Some([near, far, near_feather, far_feather]) = lod else {
        return 1.0;
    };
    if distance <= near || distance >= far {
        0.0
    } else if distance < near + near_feather {
        (distance - near) / near_feather
    } else if distance > far - far_feather {
        (far - distance) / far_feather
    } else {
        1.0
    }
}

/// `c_editable_property_base`, with what `interpolate` runs between.
#[derive(Debug, Clone)]
pub(in crate::app::model_preview) struct ParticleProperty {
    pub(super) input: ParticleInput,
    pub(super) range: ParticleInput,
    pub(super) adds: Option<bool>,
    pub(super) modifier: ParticleInput,
    pub(super) function: Arc<TagFunction>,
    pub(super) ends: [[f32; 3]; 2],
    /// What the game multiplies a value by on reading it: Halo 2's rates are
    /// a tick's, and its turns come in degrees.
    pub(super) scale: f32,
}

impl ParticleProperty {
    /// A property as its tag gives it, its inputs allocated as read: what
    /// Halo 3's and Halo 2's postprocesses do, and Reach's controllers.
    fn read(property: TagStruct<'_>, seeds: &mut SeedAllocator) -> Option<Self> {
        let mut read = Self::unallocated(property)?;
        read.allocate(seeds);
        Some(read)
    }

    /// A property with its randoms not yet given seeds, for Reach, which
    /// gathers what a definition's properties read before allocating any.
    pub(in crate::app::model_preview) fn unallocated(property: TagStruct<'_>) -> Option<Self> {
        let input = |field: &str| {
            ParticleInput::named(&property.read_enum_option(field).unwrap_or_default())
        };
        let adds = match property.read_enum_option("Output Modifier").as_deref() {
            Some("Plus") => Some(true),
            Some("Times") => Some(false),
            _ => None,
        };
        let function = tag_function_at(&property, "Mapping")?;
        Some(Self {
            input: input("Input Variable"),
            range: input("Range Variable"),
            adds,
            modifier: input("Output Modifier Input"),
            function: Arc::new(function),
            ends: read_ends(property),
            scale: 1.0,
        })
    }

    /// `c_editable_property_base`'s seeds: none for a constant; else its
    /// input, its range (Reach's only when the function is ranged), and its
    /// modifier when it has one.
    pub(super) fn allocate(&mut self, seeds: &mut SeedAllocator) {
        if let Some(taken) = seeds.taken.as_mut().filter(|taken| taken.marks_each) {
            taken.used |= self.used_states();
        }
        if self.function.as_constant().is_some() {
            return;
        }
        seeds.allocate(&mut self.input);
        if seeds.taken.is_none() || self.function.is_ranged() {
            seeds.allocate(&mut self.range);
        }
        if self.adds.is_some() {
            seeds.allocate(&mut self.modifier);
        }
    }

    /// Reach tool's `used_states`: its input's state, its range's when the
    /// function is ranged, and its modifier's when it has one.
    pub(super) fn used_states(&self) -> u32 {
        let range = if self.function.is_ranged() {
            self.range.state_bit()
        } else {
            0
        };
        let modifier = if self.adds.is_some() {
            self.modifier.state_bit()
        } else {
            0
        };
        self.input.state_bit() | range | modifier
    }

    /// The same thing, for what Halo 2 has no property for.
    pub(in crate::app::model_preview) fn fixed(value: f32) -> Option<Self> {
        use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
        let mut function = H2Function::new(FunctionType::Identity);
        function.set_clamp_range(value, value).ok()?;
        Some(Self {
            input: ParticleInput::Other,
            range: ParticleInput::Other,
            adds: None,
            modifier: ParticleInput::Other,
            function: Arc::new(
                TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).ok()?,
            ),
            ends: [[0.0; 3]; 2],
            scale: 1.0,
        })
    }

    pub(in crate::app::model_preview) fn fixed_vector(vector: [f32; 3]) -> Option<Self> {
        Some(Self {
            ends: [vector; 2],
            ..Self::fixed(1.0)?
        })
    }

    pub(in crate::app::model_preview) fn scaled(self, scale: f32) -> Self {
        Self { scale, ..self }
    }

    pub(in crate::app::model_preview) fn named(
        holder: TagStruct<'_>,
        name: &str,
        seeds: &mut SeedAllocator,
    ) -> Option<Self> {
        Self::read(holder.descend(name)?, seeds)
    }

    pub(in crate::app::model_preview) fn scalar(&self, state: &ParticleState) -> f32 {
        self.unscaled(state) * self.scale
    }

    fn unscaled(&self, state: &ParticleState) -> f32 {
        let value = self
            .function
            .evaluate(self.input.of(state), self.range.of(state));
        match self.adds {
            Some(true) => value + self.modifier.of(state),
            Some(false) => value * self.modifier.of(state),
            None => value,
        }
    }

    pub(super) fn color(&self, state: &ParticleState) -> [f32; 3] {
        let color = self
            .function
            .evaluate_color(self.input.of(state), self.range.of(state));
        [color.red, color.green, color.blue]
    }

    pub(super) fn vector(&self, state: &ParticleState) -> [f32; 3] {
        let t = self.scalar(state);
        std::array::from_fn(|axis| {
            self.ends[0][axis] + (self.ends[1][axis] - self.ends[0][axis]) * t
        })
    }

    /// `get_interpolated_value` for what a direction is: around, not across.
    pub(super) fn around(&self, state: &ParticleState) -> [f32; 3] {
        slerped(self.ends, self.scalar(state))
    }

    /// `c_editable_property_base::get_max`, sampled.
    pub(super) fn most(&self) -> f32 {
        let sample = |at: usize| {
            let t = at as f32 / 16.0;
            [
                self.function.evaluate(t, 0.0),
                self.function.evaluate(t, 1.0),
            ]
        };
        (0..=16).flat_map(sample).fold(f32::MIN, f32::max)
    }
}
