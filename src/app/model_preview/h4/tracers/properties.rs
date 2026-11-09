//! A tracer's constants, states and animated properties.

use super::*;

/// `TracerGpu::k_maxProfilesPerInstance`.
pub(super) const MOST_PROFILES: usize = 128;

/// `get_sort_bias`'s world units a step, as Reach's strips.
pub(super) const SORT_BIAS_STEP: f32 = 0.05;

/// Degrees to radians, as the game writes it.
pub(super) const RADIANS: f32 = 0.017_453_292;

/// Runtime appearance bits the postprocess adds (§5 step 7).
pub(super) const ORIGIN_FADED: u32 = 1 << 6;

pub(super) const EDGE_FADED: u32 = 1 << 7;

pub(super) const ANGLE_FADED: u32 = 1 << 9;

/// `tracerAppearanceFlags`: `random u offset`, `random v offset`.
pub(super) const RANDOM_U_OFFSET: u32 = 1 << 3;

pub(super) const RANDOM_V_OFFSET: u32 = 1 << 4;

/// `tracerStateInputEnum`, by index; `profile random` and `tracer random`
/// never reach the card, the postprocess swapping each for a seed.
pub(super) const STATE_COUNT: usize = 19;

pub(super) const PROFILE_RANDOM: u8 = 3;

const PROFILE_SEEDS: std::ops::RangeInclusive<u8> = 4..=7;

pub(super) const TRACER_RANDOM: u8 = 9;

pub(super) const TRACER_SEED_1: u8 = 10;

pub(super) const TRACER_SEED_2: u8 = 11;

pub(super) const INVALID: u8 = 18;

/// `m_usedStates &= 0x7FDF5`: not percentile, profile random or tracer random.
pub(super) const UPLOADED_STATES: u32 = 0x7_FDF5;

/// `tracerStateInputEnum`'s index for an option name.
pub(super) fn state_named(name: &str) -> u8 {
    const NAMES: [&str; STATE_COUNT - 1] = [
        "profile age",
        "profile position",
        "unused",
        "profile random",
        "profile correlation 1",
        "profile correlation 2",
        "profile correlation 3",
        "profile correlation 4",
        "game time",
        "tracer random",
        "tracer correlation 1",
        "tracer correlation 2",
        "tracer speed",
        "tracer length",
        "system age",
        "system lod",
        "effect a scale",
        "effect b scale",
    ];
    NAMES
        .iter()
        .position(|known| *known == name)
        .map_or(INVALID, |at| at as u8)
}

/// `TracerStates` and a profile's own: the value of each state by index.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct TracerState {
    pub(super) profile_age: f32,
    pub(super) percentile: f32,
    pub(super) profile_seeds: [f32; 4],
    pub(super) game_time: f32,
    pub(super) tracer_seeds: [f32; 2],
    pub(super) speed: f32,
    pub(super) length: f32,
    pub(super) system_age: f32,
    pub(super) lod: f32,
    pub(super) effect_a: f32,
    pub(super) effect_b: f32,
}

impl TracerState {
    pub(super) fn of(&self, index: u8) -> f32 {
        match index {
            0 => self.profile_age,
            1 => self.percentile,
            4..=7 => self.profile_seeds[usize::from(index - 4)],
            8 => self.game_time,
            10 | 11 => self.tracer_seeds[usize::from(index - 10)],
            12 => self.speed,
            13 => self.length,
            14 => self.system_age,
            15 => self.lod,
            16 => self.effect_a,
            17 => self.effect_b,
            _ => 0.0,
        }
    }
}

/// `tracerProperty_*`.
#[derive(Debug)]
pub(super) struct TracerProperty {
    pub(super) input: u8,
    pub(super) range: u8,
    /// `Plus`, else `Times`, by `modifier`.
    pub(super) adds: Option<bool>,
    pub(super) modifier: u8,
    pub(super) function: TagFunction,
    pub(super) ends: [[f32; 3]; 2],
}

impl TracerProperty {
    pub(super) fn read(holder: TagStruct<'_>, name: &str) -> Option<Self> {
        let property = holder.descend(name)?;
        let input =
            |field: &str| state_named(&property.read_enum_option(field).unwrap_or_default());
        Some(Self {
            input: input("Input Variable"),
            range: input("Range Variable"),
            adds: match property.read_enum_option("Output Modifier").as_deref() {
                Some("Plus") => Some(true),
                Some("Times") => Some(false),
                _ => None,
            },
            modifier: input("Output Modifier Input"),
            function: tag_function_at(&property, "Mapping")?,
            ends: read_ends(property),
        })
    }

    /// The states it reads: its input, its range when ranged, its modifier's
    /// when it has one; none for a constant.
    fn inputs(&self) -> impl Iterator<Item = u8> + '_ {
        let constant = self.function.is_constant();
        let range = self.function.is_ranged().then_some(self.range);
        let modifier = self.adds.map(|_| self.modifier);
        [Some(self.input), range, modifier]
            .into_iter()
            .flatten()
            .filter(move |_| !constant)
    }

    pub(super) fn used_states(&self) -> u32 {
        self.inputs().fold(0, |mask, input| mask | (1 << input))
    }

    /// `TracerStates::AllocateRandomSeed` (X 0x8382B928) for this property's
    /// turn: `profile random` takes the profile correlation above the highest
    /// used, `tracer random` the first tracer correlation free.
    pub(super) fn allocate_seeds(&mut self, used: &mut u32) {
        if self.function.is_constant() {
            return;
        }
        let ranged = self.function.is_ranged();
        let modified = self.adds.is_some();
        for (input, reads) in [
            (&mut self.input, true),
            (&mut self.range, ranged),
            (&mut self.modifier, modified),
        ] {
            if !reads {
                continue;
            }
            let seed = match *input {
                PROFILE_RANDOM => {
                    let profile = *used & 0xF0;
                    if profile == 0 {
                        4
                    } else {
                        (31 - profile.leading_zeros()) as u8 + 1
                    }
                }
                TRACER_RANDOM => {
                    if *used & (1 << TRACER_SEED_2) != 0 {
                        INVALID
                    } else if *used & (1 << TRACER_SEED_1) != 0 {
                        TRACER_SEED_2
                    } else {
                        TRACER_SEED_1
                    }
                }
                _ => continue,
            };
            // Past the last correlation the game logs and cycles [inferred: no kit tag does].
            let seed = if *input == PROFILE_RANDOM && !PROFILE_SEEDS.contains(&seed) {
                4
            } else {
                seed
            };
            *input = seed;
            *used |= 1 << seed;
        }
    }

    pub(super) fn scalar(&self, state: &TracerState) -> f32 {
        let value = self
            .function
            .evaluate(state.of(self.input), state.of(self.range));
        match self.adds {
            Some(true) => value + state.of(self.modifier),
            Some(false) => value * state.of(self.modifier),
            None => value,
        }
    }

    pub(super) fn color(&self, state: &TracerState) -> [f32; 3] {
        let color = self
            .function
            .evaluate_color(state.of(self.input), state.of(self.range));
        [color.red, color.green, color.blue]
    }

    /// `MapToVector2dRange`: a slerp between the two ends.
    pub(super) fn vector(&self, state: &TracerState) -> [f32; 3] {
        slerped(self.ends, self.scalar(state))
    }

    /// `get_max`: the larger end of the function's range, its low end alone
    /// for a constant.
    pub(super) fn most(&self) -> f32 {
        let (low, high) = match &self.function {
            TagFunction::Blob(blob) => {
                (blob.header().clamp_range_min, blob.header().clamp_range_max)
            }
            TagFunction::H2(h2) => (h2.clamp_range_min(), h2.clamp_range_max()),
        };
        low.max(if self.function.is_constant() {
            low
        } else {
            high
        })
    }
}

/// The tag's properties, in its order (the seed allocation's).
pub(super) const PROPERTY_NAMES: [&str; 15] = [
    "profile lifespan",
    "profile self acceleration",
    "length",
    "offset",
    "profile size",
    "profile density",
    "profile offset",
    "profile rotation",
    "profile rotation rate",
    "profile color",
    "profile alpha",
    "profile alpha2",
    "profile black point",
    "profile palette",
    "profile intensity",
];
