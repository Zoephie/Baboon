//! A light volume's animated properties: their inputs, states and seeds.

use super::load::Gen3VolumeDefinition;
use super::{Gen3Blend, high16_over_65535, lcg_step, tag_function_at};
use blam_tags::{TagFunction, TagStruct};
use std::sync::Arc;

/// `c_light_volume::k_random_seed_count`.
pub(super) const SEEDS: usize = 2;

/// `light_volume_state_input_enum`, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum Gen3VolumeInput {
    Percentile,
    GameTime,
    Age,
    /// `allocate_random_seed` swaps this for a seed before anything reads it.
    Random,
    Seed(usize),
    Lod,
    EffectA,
    EffectB,
    /// `_invalid`, which `c_light_volume_states` leaves at 0.
    Invalid,
}

impl Gen3VolumeInput {
    pub(super) fn named(name: &str) -> Self {
        match name {
            "profile position" => Self::Percentile,
            "game time" => Self::GameTime,
            "light volume age" => Self::Age,
            "light volume random" => Self::Random,
            "light volume correlation 1" => Self::Seed(0),
            "light volume correlation 2" => Self::Seed(1),
            "light volume lod" => Self::Lod,
            "effect a scale" => Self::EffectA,
            "effect b scale" => Self::EffectB,
            _ => Self::Invalid,
        }
    }

    /// `c_light_volume_definition::postprocess`: what the four properties
    /// worked out once per volume cannot read becomes `_invalid`.
    pub(super) fn per_volume(self) -> Self {
        match self {
            Self::Percentile | Self::Random => Self::Invalid,
            other => other,
        }
    }

    pub(in crate::app::model_preview) fn of(self, state: &Gen3VolumeState) -> f32 {
        match self {
            Self::Percentile => state.percentile,
            Self::GameTime => state.game_time,
            Self::Age => state.age,
            Self::Seed(seed) => state.seeds[seed],
            Self::EffectA => state.effect_a,
            Self::EffectB => state.effect_b,
            Self::Lod => state.lod,
            Self::Random | Self::Invalid => 0.0,
        }
    }
}

/// `c_light_volume_states`.
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::app::model_preview) struct Gen3VolumeState {
    pub(in crate::app::model_preview) percentile: f32,
    pub(super) game_time: f32,
    pub(super) age: f32,
    pub(super) seeds: [f32; SEEDS],
    /// Reach's state 6, `calc_lod_amount`'s; Halo 3's reads 0, the nearest
    /// of a preview's levels of detail.
    pub(super) lod: f32,
    pub(super) effect_a: f32,
    pub(super) effect_b: f32,
}

/// `c_editable_property_base`.
#[derive(Debug, Clone)]
pub(in crate::app::model_preview) struct Gen3VolumeProperty {
    pub(super) input: Gen3VolumeInput,
    pub(super) range: Gen3VolumeInput,
    /// `output_mod_enum`: added, or else multiplied, by `modifier_input`.
    pub(super) adds: Option<bool>,
    pub(super) modifier_input: Gen3VolumeInput,
    pub(super) function: Arc<TagFunction>,
}

impl Gen3VolumeProperty {
    fn read(holder: TagStruct<'_>, name: &str) -> Option<Self> {
        let property = holder.descend(name)?;
        let input = |field: &str| {
            Gen3VolumeInput::named(&property.read_enum_option(field).unwrap_or_default())
        };
        let adds = match property.read_enum_option("Output Modifier").as_deref() {
            Some("Plus") => Some(true),
            Some("Times") => Some(false),
            _ => None,
        };
        Some(Self {
            input: input("Input Variable"),
            range: input("Range Variable"),
            adds,
            modifier_input: input("Output Modifier Input"),
            function: Arc::new(tag_function_at(&property, "Mapping")?),
        })
    }

    /// What `postprocess` makes of one of the four properties worked out once
    /// per volume.
    fn per_volume(mut self) -> Self {
        self.input = self.input.per_volume();
        self.range = self.range.per_volume();
        self.modifier_input = self.modifier_input.per_volume();
        self
    }

    /// `get_uses_any_of_these_states` (tool 0x140DD1C30) for every state but
    /// the profile's place: never for a constant; else its input's, its
    /// range's when the function is ranged, and its modifier's when it has one.
    pub(super) fn reads_more_than_its_place(&self) -> bool {
        if self.function.is_constant() {
            return false;
        }
        let range = self.function.is_ranged().then_some(self.range);
        let mut inputs = [
            Some(self.input),
            range,
            self.adds.map(|_| self.modifier_input),
        ]
        .into_iter()
        .flatten();
        inputs.any(|input| input != Gen3VolumeInput::Percentile)
    }

    /// `get_max` (tool 0x140E65CE0): the larger end of the function's range,
    /// its low end alone for a constant.
    pub(super) fn most(&self) -> f32 {
        let (low, high) = match &*self.function {
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

    pub(super) fn inputs(&self) -> impl Iterator<Item = Gen3VolumeInput> {
        let modifier = self.adds.map(|_| self.modifier_input);
        [self.input, self.range].into_iter().chain(modifier)
    }

    /// `allocate_random_seed`, for this property's turn: each `random` takes
    /// the next seed, in the order input, range, modifier. A constant needs none.
    fn allocate_seeds(&mut self, next: &mut usize) {
        if self.function.as_constant().is_some() {
            return;
        }
        let modified = self.adds.is_some();
        for input in [&mut self.input, &mut self.range] {
            allocate_seed(input, next);
        }
        if modified {
            allocate_seed(&mut self.modifier_input, next);
        }
    }

    pub(in crate::app::model_preview) fn scalar(&self, state: &Gen3VolumeState) -> f32 {
        let value = self
            .function
            .evaluate(self.input.of(state), self.range.of(state));
        let modifier = self.modifier_input.of(state);
        match self.adds {
            Some(true) => value + modifier,
            Some(false) => value * modifier,
            None => value,
        }
    }

    /// `get_max` over the mean of `get_min` and it, sampled.
    pub(super) fn peak_over_mean(&self) -> f32 {
        let sample = |at: usize| {
            let t = at as f32 / 16.0;
            [
                self.function.evaluate(t, 0.0),
                self.function.evaluate(t, 1.0),
            ]
        };
        let (mut least, mut most) = (f32::MAX, f32::MIN);
        for value in (0..=16).flat_map(sample) {
            least = least.min(value);
            most = most.max(value);
        }
        most / ((least + most) * 0.5)
    }

    pub(in crate::app::model_preview) fn color(&self, state: &Gen3VolumeState) -> [f32; 3] {
        let color = self
            .function
            .evaluate_color(self.input.of(state), self.range.of(state));
        [color.red, color.green, color.blue]
    }
}

pub(super) fn allocate_seed(input: &mut Gen3VolumeInput, next: &mut usize) {
    if *input == Gen3VolumeInput::Random {
        *input = Gen3VolumeInput::Seed(*next % SEEDS);
        *next += 1;
    }
}

/// The first seed free: past the highest a tag named for itself.
pub(super) fn first_free_seed(properties: &[&Gen3VolumeProperty]) -> usize {
    let named = properties.iter().flat_map(|property| property.inputs());
    named
        .filter_map(|input| {
            if let Gen3VolumeInput::Seed(seed) = input {
                Some(seed + 1)
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0)
}

/// `real_internal_random_range`'s generator, for a volume's own seeds.
pub(super) fn volume_seeds(index: usize) -> [f32; SEEDS] {
    let first = lcg_step(index as u32);
    [first, lcg_step(first)].map(high16_over_65535)
}

/// The eight properties of a `light_volumes` element, as read.
pub(super) struct VolumeProperties {
    pub(super) length: Gen3VolumeProperty,
    offset: Gen3VolumeProperty,
    pub(super) density: Gen3VolumeProperty,
    profile_length: Gen3VolumeProperty,
    thickness: Gen3VolumeProperty,
    color: Gen3VolumeProperty,
    alpha: Gen3VolumeProperty,
    intensity: Gen3VolumeProperty,
}

impl VolumeProperties {
    pub(super) fn read(element: TagStruct<'_>) -> Option<Self> {
        let each = |name: &str| Gen3VolumeProperty::read(element, name);
        Some(Self {
            length: each("length")?,
            offset: each("offset")?,
            density: each("profile_density")?,
            profile_length: each("profile_length")?,
            thickness: each("profile_thickness")?,
            color: each("profile_color")?,
            alpha: each("profile_alpha")?,
            intensity: each("profile_intensity")?,
        })
    }

    pub(super) fn all(&self) -> [&Gen3VolumeProperty; 8] {
        [
            &self.length,
            &self.offset,
            &self.density,
            &self.profile_length,
            &self.thickness,
            &self.color,
            &self.alpha,
            &self.intensity,
        ]
    }

    /// `postprocess`'s brightness ratio, narrowed inputs and seeds, into
    /// the definition `rest` fills out.
    pub(super) fn postprocessed(
        self,
        element: TagStruct<'_>,
        rest: impl FnOnce(Gen3VolumeDefinition) -> Gen3VolumeDefinition,
    ) -> Gen3VolumeDefinition {
        let Self {
            length,
            offset,
            density,
            profile_length,
            mut thickness,
            mut color,
            mut alpha,
            mut intensity,
        } = self;
        // With none given, what the functions themselves make of it. A tag
        // older than the field has none either.
        let brightness_ratio = element
            .read_real("brightness ratio")
            .filter(|ratio| *ratio != 0.0)
            .unwrap_or_else(|| alpha.peak_over_mean() * intensity.peak_over_mean());
        let [length, offset, density, profile_length] =
            [length, offset, density, profile_length].map(Gen3VolumeProperty::per_volume);
        let mut next = first_free_seed(&[
            &length,
            &offset,
            &density,
            &profile_length,
            &thickness,
            &color,
            &alpha,
            &intensity,
        ]);
        for property in [&mut thickness, &mut color, &mut alpha, &mut intensity] {
            property.allocate_seeds(&mut next);
        }
        rest(Gen3VolumeDefinition {
            length,
            offset,
            density,
            profile_length,
            thickness,
            color,
            alpha,
            intensity,
            brightness_ratio,
            blend: Gen3Blend::Additive,
            bitmap: None,
            sampler: None,
            sort_layer: 2,
            reach: false,
            circular: None,
            depth_fade: None,
            lod: None,
            precompiled: None,
            h4: None,
        })
    }
}
