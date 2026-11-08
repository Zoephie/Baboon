//! Halo 3's light volumes: a row of camera-facing profiles along a direction.
//! The game works these out on the card every frame, but nothing in one
//! depends on the last, so they are worked out here a frame at a time.
//! Halo 4 draws them through their material from each profile's packed
//! state (`h4_light_volume_pass`), which is worked out here the same way.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use super::super::attachment::MarkerFrame;
use super::super::common::fields::{elements, has_flag};
use super::super::antennas::AntennaQuad;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::functions::FunctionValues;
use super::super::common::named_function::tag_function_at;
use super::super::effects::{
    EffectTimes, for_each_attached_part, part_frame, read_part_placement,
};
use super::super::particles::inline_real;
use super::shaders::{H3Blend, H3Sampler, read_h3_definition};
use super::strips::EPSILON;
use super::super::h4::particles::{f32_to_f16, pack_argb8};
use super::super::h4::materials::{H4EffectMaterial, resolve_h4_effect_material};
use super::super::gen3::materials::{H3Bitmap, read_h3_bitmap, resolve_embedded_method};
use super::super::renderer::{H3_EXPOSURE, reach_alt_exposure};
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{clamped_unit, cross, length};
use crate::app::model_preview::common::random::{high16_over_65535, lcg_step};
use crate::app::model_preview::common::attached::attached;

/// What a volume's row can hold, whatever its tag says; also the most
/// Reach's postprocess bakes.
const MOST_PROFILES: i32 = 128;
/// Reach's postprocess (tool 0x140DD0FA0): an `LOD out distance` of none
/// is this far.
const DEFAULT_LOD_OUT: f32 = 20.0;
/// `c_light_volume::k_random_seed_count`.
const SEEDS: usize = 2;

/// `light_volume_state_input_enum`, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H3VolumeInput {
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

impl H3VolumeInput {
    fn named(name: &str) -> Self {
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
    fn per_volume(self) -> Self {
        match self {
            Self::Percentile | Self::Random => Self::Invalid,
            other => other,
        }
    }

    fn of(self, state: &H3VolumeState) -> f32 {
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
struct H3VolumeState {
    percentile: f32,
    game_time: f32,
    age: f32,
    seeds: [f32; SEEDS],
    /// Reach's state 6, `calc_lod_amount`'s; Halo 3's reads 0, the nearest
    /// of a preview's levels of detail.
    lod: f32,
    effect_a: f32,
    effect_b: f32,
}

/// `c_editable_property_base`.
#[derive(Debug, Clone)]
struct H3VolumeProperty {
    input: H3VolumeInput,
    range: H3VolumeInput,
    /// `output_mod_enum`: added, or else multiplied, by `modifier_input`.
    adds: Option<bool>,
    modifier_input: H3VolumeInput,
    function: Arc<TagFunction>,
}

impl H3VolumeProperty {
    fn read(holder: TagStruct<'_>, name: &str) -> Option<Self> {
        let property = holder.descend(name)?;
        let input = |field: &str| H3VolumeInput::named(&property.read_enum_option(field).unwrap_or_default());
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
    fn reads_more_than_its_place(&self) -> bool {
        if self.function.is_constant() {
            return false;
        }
        let range = self.function.is_ranged().then_some(self.range);
        let mut inputs = [Some(self.input), range, self.adds.map(|_| self.modifier_input)].into_iter().flatten();
        inputs.any(|input| input != H3VolumeInput::Percentile)
    }

    /// `get_max` (tool 0x140E65CE0): the larger end of the function's range,
    /// its low end alone for a constant.
    fn most(&self) -> f32 {
        let (low, high) = match &*self.function {
            TagFunction::Blob(blob) => (blob.header().clamp_range_min, blob.header().clamp_range_max),
            TagFunction::H2(h2) => (h2.clamp_range_min(), h2.clamp_range_max()),
        };
        low.max(if self.function.is_constant() { low } else { high })
    }

    fn inputs(&self) -> impl Iterator<Item = H3VolumeInput> {
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

    fn scalar(&self, state: &H3VolumeState) -> f32 {
        let value = self.function.evaluate(self.input.of(state), self.range.of(state));
        let modifier = self.modifier_input.of(state);
        match self.adds {
            Some(true) => value + modifier,
            Some(false) => value * modifier,
            None => value,
        }
    }

    /// `get_max` over the mean of `get_min` and it, sampled.
    fn peak_over_mean(&self) -> f32 {
        let sample = |at: usize| {
            let t = at as f32 / 16.0;
            [self.function.evaluate(t, 0.0), self.function.evaluate(t, 1.0)]
        };
        let (mut least, mut most) = (f32::MAX, f32::MIN);
        for value in (0..=16).flat_map(sample) {
            least = least.min(value);
            most = most.max(value);
        }
        most / ((least + most) * 0.5)
    }

    fn color(&self, state: &H3VolumeState) -> [f32; 3] {
        let color = self.function.evaluate_color(self.input.of(state), self.range.of(state));
        [color.red, color.green, color.blue]
    }
}

fn allocate_seed(input: &mut H3VolumeInput, next: &mut usize) {
    if *input == H3VolumeInput::Random {
        *input = H3VolumeInput::Seed(*next % SEEDS);
        *next += 1;
    }
}

/// The first seed free: past the highest a tag named for itself.
fn first_free_seed(properties: &[&H3VolumeProperty]) -> usize {
    let named = properties.iter().flat_map(|property| property.inputs());
    named.filter_map(|input| if let H3VolumeInput::Seed(seed) = input { Some(seed + 1) } else { None }).max().unwrap_or(0)
}

/// `real_internal_random_range`'s generator, for a volume's own seeds.
fn volume_seeds(index: usize) -> [f32; SEEDS] {
    let first = lcg_step(index as u32);
    [first, lcg_step(first)].map(high16_over_65535)
}

/// Reach's `LOD in distance` and the rest, as `postprocess` (tool
/// 0x140DD0FA0) leaves them, for `calc_lod_amount`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ReachVolumeLod {
    near: f32,
    feather_in: f32,
    inverse_in: f32,
    far: f32,
    feather_out: f32,
    inverse_out: f32,
    /// `lod always 1.0`, which with `lod enabled` draws nothing.
    always_one: bool,
}

impl ReachVolumeLod {
    /// With `lod enabled` only; the inverses are the postprocess's, which a
    /// tag on disk holds 0 for.
    fn read(element: TagStruct<'_>, flagged: impl Fn(&str) -> bool) -> Option<Self> {
        if !flagged("lod enabled") {
            return None;
        }
        let real = |name: &str| element.read_real(name).unwrap_or(0.0);
        let near = [real("LOD in distance"), real("LOD feather in delta")];
        let far = [real("LOD out distance"), real("LOD feather out delta")];
        Some(Self::postprocessed(near, far, flagged("lod always 1.0")))
    }

    /// `postprocess` (reach-tool `light_volume_definitions.cpp` l.175–195):
    /// an out distance of none is 20, an in distance at least epsilon, and
    /// each inverse over a feather of at least epsilon.
    fn postprocessed([near, feather_in]: [f32; 2], [far, feather_out]: [f32; 2], always_one: bool) -> Self {
        Self {
            near: near.max(EPSILON),
            feather_in,
            inverse_in: feather_in.max(EPSILON).recip(),
            far: if far.abs() < EPSILON { DEFAULT_LOD_OUT } else { far },
            feather_out,
            inverse_out: feather_out.max(EPSILON).recip(),
            always_one,
        }
    }
}

/// `c_light_volume_definition::calc_lod_amount` (the 360's 0x82EC77D0, MCC
/// sub_14029B0D0) at `distance` from the eye. `lod same in splitscreen`
/// only skips `x_distance_lod_scale`, which is 1 for one view \[inferred\].
fn lod_amount(lod: Option<&ReachVolumeLod>, distance: f32) -> f32 {
    let Some(lod) = lod else {
        return 1.0;
    };
    // Sic: with `lod enabled`, `always 1.0` comes out 0, in both builds.
    if lod.always_one || distance <= lod.near || distance >= lod.far {
        return 0.0;
    }
    if distance < lod.near + lod.feather_in {
        (distance - lod.near) * lod.inverse_in
    } else if distance > lod.far - lod.feather_out {
        (lod.far - distance) * lod.inverse_out
    } else {
        1.0
    }
}

/// Reach's `max profile count` (tool 0x140DD0FA0): the most profiles the
/// two per-volume settings can make, 0 to 128, a NaN none.
fn max_profile_count(density: &H3VolumeProperty, length: &H3VolumeProperty) -> usize {
    let most = (density.most() * length.most()).max(0.0);
    if !(MOST_PROFILES as f32 > most) { MOST_PROFILES as usize } else { most as i64 as usize }
}

/// `postprocess_build_precompiled_verts_if_needed` (tool 0x140DD2080, the
/// 360's 0x82EC6E58)'s test: unless `disable precompiled profiles`, a volume
/// that `force precompile profiles` or whose eight properties read nothing
/// but the profile's place bakes its rows, if it can make any.
fn precompiles(flagged: impl Fn(&str) -> bool, properties: &[&H3VolumeProperty; 8], most: usize) -> bool {
    let profile_only = properties.iter().all(|property| !property.reads_more_than_its_place());
    !flagged("disable precompiled profiles") && (flagged("force precompile profiles") || profile_only) && most != 0
}

/// One of a `light_volume_system`'s `light_volumes`.
#[derive(Debug)]
pub(crate) struct H3VolumeDefinition {
    length: H3VolumeProperty,
    offset: H3VolumeProperty,
    density: H3VolumeProperty,
    profile_length: H3VolumeProperty,
    thickness: H3VolumeProperty,
    color: H3VolumeProperty,
    alpha: H3VolumeProperty,
    intensity: H3VolumeProperty,
    brightness_ratio: f32,
    blend: H3Blend,
    /// None for Reach's `circular`, which samples no map.
    bitmap: Option<H3Bitmap>,
    sampler: Option<H3Sampler>,
    sort_layer: i32,
    /// Reach's: its own colour, exposure and level of detail.
    reach: bool,
    /// Reach's `albedo` `circular`: `center_offset` and `falloff`.
    circular: Option<[f32; 2]>,
    /// Reach's `depth_fade` `on`: `depth_fade_range`.
    depth_fade: Option<f32>,
    lod: Option<ReachVolumeLod>,
    /// Reach's precompiled rows (`s_light_volume_pre_vertex`), at
    /// `max profile count`: each profile's colour times its intensity
    /// squared times its alpha, and its thickness.
    precompiled: Option<Vec<([f32; 3], f32)>>,
    /// Halo 4's: the `actual material` it draws through, and its
    /// `appearance flags`.
    h4: Option<(H4EffectMaterial, u32)>,
}

impl H3VolumeDefinition {
    /// The baked rows: profile `i` at `i / (max − 0.5)` with every other
    /// state 0 (a fresh `c_light_volume_states`), unclamped.
    fn bake(&self, most: usize) -> Vec<([f32; 3], f32)> {
        let place = |index: usize| index as f32 / ((most as f32 + 0.5) - 1.0);
        (0..most)
            .map(|index| {
                let state = H3VolumeState { percentile: place(index), ..Default::default() };
                let intensity = self.intensity.scalar(&state);
                let factor = intensity * intensity * self.alpha.scalar(&state);
                (self.color.color(&state).map(|channel| channel * factor), self.thickness.scalar(&state))
            })
            .collect()
    }

    /// `light_volume.hlsl`'s profile colour, before the map: Halo 3's colour
    /// at its exposure with alpha unscaled by intensity; Reach's
    /// (`lights__light_volume.hlsl`) with alpha by intensity too and its
    /// colour at `V_ILLUM_EXPOSURE` unless it multiplies, or a baked row's,
    /// which carries both and an alpha and intensity of 1. Alpha goes by the
    /// brightness `ratio` over the `overdraw`.
    fn profile_color(
        &self,
        state: &H3VolumeState,
        baked: Option<[f32; 3]>,
        [ratio, overdraw]: [f32; 2],
        stops: f32,
    ) -> [f32; 4] {
        if !self.reach {
            // `light_volume_fx.hlsl`'s `STATE.m_color * STATE.m_intensity` is a
            // float4, so intensity scales alpha too (Halo 3 and ODST alike).
            let color = self.color.color(state).map(|channel| channel.clamp(0.0, 1.0));
            let intensity = self.intensity.scalar(state);
            let [red, green, blue] = color.map(|channel| channel * intensity * H3_EXPOSURE);
            let alpha = self.alpha.scalar(state).clamp(0.0, 1.0) * intensity;
            return [red, green, blue, alpha * ratio / overdraw];
        }
        let exposure = if self.blend == H3Blend::Multiply { 1.0 } else { reach_alt_exposure(stops)[1] };
        // TODO(reach-effects spec §3.4): Reach premultiplies and writes an
        // alpha of 1, which the flare program's `(rgb, a)` under SRC_ALPHA,
        // ONE matches; an alpha-blended volume (none in the kit) would not.
        let ([red, green, blue], alpha) = match baked {
            Some(baked) => (baked, 1.0),
            None => {
                let intensity = self.intensity.scalar(state);
                let color = self.color.color(state).map(|channel| channel.clamp(0.0, 1.0) * intensity);
                (color, self.alpha.scalar(state).clamp(0.0, 1.0) * intensity)
            }
        };
        [red * exposure, green * exposure, blue * exposure, alpha * ratio / overdraw]
    }
}

/// What a volume's frame lays out before its profiles.
struct VolumeLayout {
    state: H3VolumeState,
    origin: [f32; 3],
    direction: [f32; 3],
    count: i32,
    /// `length × density`, whole or not.
    profiles: f32,
    /// `1 / density`.
    distance: f32,
    offset: f32,
    side_length: f32,
}

/// `light_volume_state_input_enum`'s order, the order of
/// `LightVolumeOverallState.inputs`; `allocate_random_seed` leaves nothing
/// reading `random`.
const H4_STATE_ORDER: [H3VolumeInput; 10] = [
    H3VolumeInput::Percentile,
    H3VolumeInput::GameTime,
    H3VolumeInput::Age,
    H3VolumeInput::Random,
    H3VolumeInput::Seed(0),
    H3VolumeInput::Seed(1),
    H3VolumeInput::Lod,
    H3VolumeInput::EffectA,
    H3VolumeInput::EffectB,
    H3VolumeInput::Invalid,
];

/// `ProfileMemexportedState`, as `light_volume_main` writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct H4ProfileState {
    position: [f32; 3],
    percentile: f32,
    /// Unclamped; the packing saturates it.
    color: [f32; 4],
    thickness: f32,
    intensity: f32,
}

/// `WriteProfileMemexportedState`: `RawProfileState`'s eight words, the
/// thickness and intensity as halves, the colour as ARGB8, the pad 0.
fn pack_profile_state(state: &H4ProfileState) -> [u32; 8] {
    let [x, y, z] = state.position.map(f32::to_bits);
    let misc = f32_to_f16(state.thickness) | (f32_to_f16(state.intensity) << 16);
    [x, y, z, state.percentile.to_bits(), misc, pack_argb8(state.color), 0, 0]
}

/// One Halo 4 volume's frame: `LightVolumeOverallState` and its profiles'
/// packed states, in row order.
#[derive(Debug, Clone)]
pub(crate) struct H4VolumeDraw {
    pub(crate) appearance: f32,
    pub(crate) brightness_ratio: f32,
    pub(crate) offset: f32,
    /// `numProfiles`: `length × density`, whole or not.
    pub(crate) profiles: f32,
    /// `profileDistance`.
    pub(crate) distance: f32,
    /// `profileLength`.
    pub(crate) side_length: f32,
    pub(crate) origin: [f32; 3],
    pub(crate) direction: [f32; 3],
    pub(crate) inputs: [f32; 10],
    pub(crate) states: Vec<[u32; 8]>,
}

/// A volume at a marker of the object it is attached to.
#[derive(Debug, Clone)]
pub(crate) struct H3LightVolumeAt {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    /// `use parent position but world orientation`: its forward does not turn.
    world_oriented: bool,
    only_under: Option<(String, String)>,
    scale_by: String,
    /// `secondary scale`, the effect's `b`.
    secondary_by: String,
    seeds: [f32; SEEDS],
    times: Arc<EffectTimes>,
    event: usize,
    pub(crate) definition: Arc<H3VolumeDefinition>,
}

fn read_definition(reader: &TagReader, element: TagStruct<'_>) -> Option<H3VolumeDefinition> {
    if reader.source.game() == Some(GameId::Halo4) {
        return read_h4_definition(reader, element);
    }
    let reach = reader.source.game() == Some(GameId::HaloReach);
    let shader = element.descend("actual shader?")?;
    let definition = shader.read_tag_ref_path("definition").filter(|path| !path.is_empty())?;
    let definition = read_h3_definition(reader, &definition)?;
    let options: Vec<usize> = elements(shader, "options")
        .iter()
        .map(|option| option.read_int_any("short").unwrap_or(0).max(0) as usize)
        .collect();
    let chosen = |category: &str| definition.chosen(category, &options).unwrap_or_default().to_owned();
    let resolved = resolve_embedded_method(reader, shader);
    let resolved_real = |name: &str, default: f32| inline_real(resolved.as_ref(), name).unwrap_or(default);
    // `lights__light_volume.hlsl` l.288–296; `albedo_circular.render_method_option`'s defaults.
    let circular = (reach && chosen("albedo") == "circular")
        .then(|| [resolved_real("center_offset", 1.1), resolved_real("falloff", 2.0)]);
    let (bitmap, sampler) = if circular.is_some() {
        (None, None)
    } else {
        let parameters = elements(shader, "parameters");
        let base = parameters.iter().find(|parameter| {
            parameter.read_string_id("parameter name").as_deref() == Some("base_map")
        })?;
        let map = base.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
        let baked = resolved
            .as_ref()
            .and_then(|resolved| super::super::gen3::materials::read_resolved_map(reader, resolved, "base_map"));
        match baked {
            Some((bitmap, sampler)) => (Some(bitmap), Some(sampler)),
            None => (Some(read_h3_bitmap(reader, &map, 0, SamplerKind::Texture2D)?), None),
        }
    };
    let blend = definition.blend(&options);
    // `depth_fade_on.render_method_option`'s range. `biased`'s option tag is
    // not in the kit, and no volume uses it.
    let depth_fade = (reach && chosen("depth_fade") == "on" && blend != H3Blend::Opaque)
        .then(|| resolved_real("depth_fade_range", 0.1).max(1e-4));
    let flagged = |wanted: &str| has_flag(element, "flags", wanted);
    let lod = if reach { ReachVolumeLod::read(element, flagged) } else { None };
    let properties = VolumeProperties::read(element)?;
    // The postprocess tests what the properties read before it narrows the
    // per-volume four's inputs.
    let most = max_profile_count(&properties.density, &properties.length);
    let precompiled = reach && precompiles(flagged, &properties.all(), most);
    let mut volume = properties.postprocessed(element, |properties| H3VolumeDefinition {
        blend,
        sampler,
        bitmap,
        sort_layer: super::super::particles::h3_sort_layer(shader),
        reach,
        circular,
        depth_fade,
        lod,
        precompiled: None,
        h4: None,
        ..properties
    });
    if precompiled {
        volume.precompiled = Some(volume.bake(most));
    }
    Some(volume)
}

/// Halo 4's `c_light_volume_definition::postprocess_block_proc` (MCC
/// 0x1403ABF50): Reach's, through the `actual material`, and never
/// precompiled, which MCC clears every time and could not draw ("not
/// implemented for D3D11").
fn read_h4_definition(reader: &TagReader, element: TagStruct<'_>) -> Option<H3VolumeDefinition> {
    let material = blam_tags::material::Material::from_struct(&element.descend("actual material?")?);
    let resolved = resolve_h4_effect_material(reader, &material, "light_volume")
        .map_err(|error| eprintln!("h4 light volume material: {error}"))
        .ok()?;
    let flagged = |wanted: &str| has_flag(element, "flags", wanted);
    let lod = ReachVolumeLod::read(element, flagged);
    let appearance = element.read_int_any("appearance flags").unwrap_or(0) as u32;
    let properties = VolumeProperties::read(element)?;
    Some(properties.postprocessed(element, |properties| H3VolumeDefinition {
        // The material blends; this only sorts.
        blend: H3Blend::Additive,
        sampler: None,
        bitmap: None,
        sort_layer: i32::from(material.sort_layer).clamp(1, 3),
        reach: true,
        circular: None,
        depth_fade: None,
        lod,
        precompiled: None,
        h4: Some((resolved, appearance)),
        ..properties
    }))
}

/// The eight properties of a `light_volumes` element, as read.
struct VolumeProperties {
    length: H3VolumeProperty,
    offset: H3VolumeProperty,
    density: H3VolumeProperty,
    profile_length: H3VolumeProperty,
    thickness: H3VolumeProperty,
    color: H3VolumeProperty,
    alpha: H3VolumeProperty,
    intensity: H3VolumeProperty,
}

impl VolumeProperties {
    fn read(element: TagStruct<'_>) -> Option<Self> {
        let each = |name: &str| H3VolumeProperty::read(element, name);
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

    fn all(&self) -> [&H3VolumeProperty; 8] {
        [
            &self.length, &self.offset, &self.density, &self.profile_length,
            &self.thickness, &self.color, &self.alpha, &self.intensity,
        ]
    }

    /// `postprocess`'s brightness ratio, narrowed inputs and seeds, into
    /// the definition `rest` fills out.
    fn postprocessed(
        self,
        element: TagStruct<'_>,
        rest: impl FnOnce(H3VolumeDefinition) -> H3VolumeDefinition,
    ) -> H3VolumeDefinition {
        let Self { length, offset, density, profile_length, mut thickness, mut color, mut alpha, mut intensity } = self;
        // With none given, what the functions themselves make of it. A tag
        // older than the field has none either.
        let brightness_ratio = element
            .read_real("brightness ratio")
            .filter(|ratio| *ratio != 0.0)
            .unwrap_or_else(|| alpha.peak_over_mean() * intensity.peak_over_mean());
        let [length, offset, density, profile_length] =
            [length, offset, density, profile_length].map(H3VolumeProperty::per_volume);
        let mut next =
            first_free_seed(&[&length, &offset, &density, &profile_length, &thickness, &color, &alpha, &intensity]);
        for property in [&mut thickness, &mut color, &mut alpha, &mut intensity] {
            property.allocate_seeds(&mut next);
        }
        rest(H3VolumeDefinition {
            length,
            offset,
            density,
            profile_length,
            thickness,
            color,
            alpha,
            intensity,
            brightness_ratio,
            blend: H3Blend::Additive,
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

/// The volumes an object's attached effects run, one at each marker of a
/// location.
pub(in crate::app::model_preview) fn read_h3_light_volumes(
    reader: &TagReader,
    object: &blam_tags::TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3LightVolumeAt> {
    let mut volumes = Vec::new();
    let volume_group = u32::from_be_bytes(*b"ltvl");
    for_each_attached_part(reader, object, |part| {
        if part.group != volume_group {
            return;
        }
        let Ok(tag) = reader.load(&part.path, "light_volume_system", b"ltvl") else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for element in elements(tag.root(), "light_volumes") {
            let Some(definition) = read_definition(reader, element).map(Arc::new) else {
                continue;
            };
            for marker in &part.effect.locations(markers, &part.marker, &frame) {
                let (place, node) = frame(marker);
                let (place, node) = part.placed(place, node);
                let place = part_frame(&place, offset, yaw, pitch);
                volumes.push(H3LightVolumeAt {
                    object: owner,
                    node,
                    position: place.position,
                    forward: place.axes[0],
                    world_oriented: part.world_oriented,
                    only_under: marker.only_under.clone(),
                    scale_by: part.effect.scale_by.clone(),
                    secondary_by: part.effect.secondary_by.clone(),
                    seeds: volume_seeds(volumes.len()),
                    times: Arc::clone(&part.times),
                    event: part.event,
                    definition: Arc::clone(&definition),
                });
            }
        }
    });
    volumes
}




/// `c_light_volume_gpu::render`'s count, and the two sizes the shaders get.
fn profile_layout(length: f32, density: f32) -> (i32, f32, f32) {
    if length <= 0.0 || density <= 0.0 {
        return (0, 0.0, 0.0);
    }
    let count = length * density;
    ((count as i32).min(MOST_PROFILES), count, 1.0 / density)
}

/// `light_volume_update`'s place along the row. A row of one sits at its start.
fn percentile(index: i32, profiles: f32) -> f32 {
    if profiles > 1.0 { index as f32 / (profiles - 1.0) } else { 0.0 }
}

/// What `light_volume.fx` divides alpha by to hold a volume's brightness at
/// any angle. Head on the spacing vanishes, and every profile stacks.
fn overdraw(profiles: f32, profile_length: f32, spacing: f32) -> f32 {
    if spacing > 0.0 { profiles.min(profile_length / spacing) } else { profiles }.max(1e-6)
}

impl H3LightVolumeAt {
    /// The functions it reads off its object.
    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with time alone.
    pub(crate) fn animates(&self) -> bool {
        let volume = &*self.definition;
        let reads_time = [
            &volume.length, &volume.offset, &volume.density, &volume.profile_length,
            &volume.thickness, &volume.color, &volume.alpha, &volume.intensity,
        ];
        self.times.moves()
            || reads_time.iter().any(|property| property.inputs().any(|input| input == H3VolumeInput::GameTime))
    }

    /// Its sort position: `submit` hands the transparents its location.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        posed_on(self.node, self.position, self.forward, bones).0
    }

    pub(crate) fn sampler(&self) -> Option<H3Sampler> {
        self.definition.sampler
    }

    pub(crate) fn bitmap(&self) -> Option<&H3Bitmap> {
        self.definition.bitmap.as_ref()
    }

    /// Reach's `circular` albedo's `center_offset` and `falloff`.
    pub(crate) fn circular(&self) -> Option<[f32; 2]> {
        self.definition.circular
    }

    /// Reach's `depth_fade_range`, when it fades against the scene.
    pub(crate) fn depth_fade(&self) -> Option<f32> {
        self.definition.depth_fade
    }

    pub(crate) fn sort_layer(&self) -> i32 {
        self.definition.sort_layer
    }

    pub(crate) fn blend(&self) -> H3Blend {
        self.definition.blend
    }

    /// The states this frame, or none while its event is not running.
    fn state(&self, values: Option<&FunctionValues>, time: f32, onset: f32) -> Option<H3VolumeState> {
        let read = |name: &str, none: f32| {
            if name.is_empty() {
                return none;
            }
            values.and_then(|values| values.get(name)).copied().unwrap_or(0.0)
        };
        // `attachments_update` runs the effect while its function is on.
        let effect_a = read(&self.scale_by, 1.0);
        if effect_a <= 0.0 {
            return None;
        }
        Some(H3VolumeState {
            percentile: 0.0,
            game_time: time.rem_euclid(1.0),
            age: self.times.age(self.event, time - onset)?,
            seeds: self.seeds,
            lod: 0.0,
            effect_a,
            effect_b: read(&self.secondary_by, 1.0),
        })
    }

    /// What `c_light_volume_gpu::render` works out once a frame, or none
    /// while it draws nothing.
    fn layout(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Option<VolumeLayout> {
        let mut state = self.state(values, time, onset)?;
        let volume = &*self.definition;
        let (origin, turned) = posed_on(self.node, self.position, self.forward, bones);
        if volume.reach {
            // `c_light_volume_gpu::render` (Reach: the 360's 0x826B80B8, MCC
            // sub_140887B70; Halo 4: MCC 0x1408ECFD0): by the eye's whole
            // distance from the location, nothing at no amount, and the
            // amount is state 6.
            let from_eye = std::array::from_fn(|axis| origin[axis] - view.eye[axis]);
            let amount = lod_amount(volume.lod.as_ref(), length(from_eye));
            if amount <= 0.0 {
                return None;
            }
            state.lod = amount;
        }
        let length = volume.length.scalar(&state);
        let density = volume.density.scalar(&state);
        let (count, profiles, distance) = profile_layout(length, density);
        let direction = if self.world_oriented { self.forward } else { turned };
        Some(VolumeLayout {
            state,
            origin,
            direction: clamped_unit(direction),
            count,
            profiles,
            distance,
            offset: volume.offset.scalar(&state),
            side_length: volume.profile_length.scalar(&state),
        })
    }

    /// Halo 4's draw this frame: `SetMaterialState`'s overall state and,
    /// for each profile, the state `light_volume_update.fx` writes, packed.
    pub(crate) fn h4_draw(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Option<H4VolumeDraw> {
        let (_, appearance) = self.definition.h4.as_ref()?;
        self.h4_frame(*appearance, values, bones, view, time, onset)
    }

    /// [`Self::h4_draw`] with `appearance flags` of `appearance`.
    fn h4_frame(
        &self,
        appearance: u32,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Option<H4VolumeDraw> {
        let volume = &*self.definition;
        let VolumeLayout { mut state, origin, direction, count, profiles, distance, offset, side_length } =
            self.layout(values, bones, view, time, onset)?;
        if count <= 0 {
            return None;
        }
        // `inputs[]`: every state but the profile's own place, which the
        // update reads from the profile.
        let inputs = H4_STATE_ORDER.map(|input| match input {
            H3VolumeInput::Percentile => 0.0,
            other => other.of(&state),
        });
        let states = (0..count)
            .map(|index| {
                state.percentile = percentile(index, profiles);
                let along = offset + distance * index as f32;
                let position: [f32; 3] = std::array::from_fn(|axis| origin[axis] + direction[axis] * along);
                let [red, green, blue] = volume.color.color(&state);
                pack_profile_state(&H4ProfileState {
                    position,
                    percentile: state.percentile,
                    color: [red, green, blue, volume.alpha.scalar(&state)],
                    thickness: volume.thickness.scalar(&state),
                    intensity: volume.intensity.scalar(&state),
                })
            })
            .collect();
        Some(H4VolumeDraw {
            appearance: appearance as f32,
            brightness_ratio: volume.brightness_ratio,
            offset,
            profiles,
            distance,
            side_length,
            origin,
            direction,
            inputs,
            states,
        })
    }

    /// Halo 4's material, when it is Halo 4's.
    pub(crate) fn h4_material(&self) -> Option<&H4EffectMaterial> {
        self.definition.h4.as_ref().map(|(material, _)| material)
    }

    /// Each profile's quad this frame, nearest the marker first, at the area
    /// screen effects' exposure `stops`.
    pub(crate) fn quads(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<AntennaQuad> {
        let Some(VolumeLayout { mut state, origin, direction, count, profiles, distance, offset, side_length }) =
            self.layout(values, bones, view, time, onset)
        else {
            return Vec::new();
        };
        let volume = &*self.definition;
        // A baked volume draws `max profile count` rows, spaced and overdrawn
        // by the live settings; that it draws none where they make none was
        // not traced.
        let baked = volume.precompiled.as_deref().filter(|_| count > 0);
        let count = baked.map_or(count, |rows| rows.len() as i32);
        let mut quads = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            state.percentile = percentile(index, profiles);
            let along = offset + distance * index as f32;
            let place: [f32; 3] = std::array::from_fn(|axis| origin[axis] + direction[axis] * along);
            let row = baked.map(|rows| rows[index as usize]);
            let thickness = row.map_or_else(|| volume.thickness.scalar(&state), |(_, thickness)| thickness);
            let to_profile = clamped_unit(std::array::from_fn(|axis| place[axis] - view.eye[axis]));
            let sine = length(cross(direction, to_profile)).min(1.0);
            let drawn_length = thickness + (side_length - thickness) * sine;
            let up = clamped_unit(cross(to_profile, direction));
            let across = cross(to_profile, up);
            let corner = |u: f32, v: f32| {
                let (x, y) = ((u - 0.5) * drawn_length, (v - 0.5) * thickness);
                let [a, b, c] = std::array::from_fn(|axis| place[axis] + across[axis] * x + up[axis] * y);
                [a, b, c, u, v]
            };
            let ratio = volume.brightness_ratio + (1.0 - volume.brightness_ratio) * sine;
            let overdrawn = overdraw(profiles, drawn_length, distance * sine);
            quads.push(AntennaQuad {
                corners: [corner(0.0, 0.0), corner(1.0, 0.0), corner(0.0, 1.0), corner(1.0, 1.0)],
                color: volume.profile_color(&state, row.map(|(color, _)| color), [ratio, overdrawn], stops),
            });
        }
        quads
    }
}

attached!(H3LightVolumeAt);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::game::GameId;

    #[test]
    fn the_count_truncates_while_the_shaders_keep_the_whole_product() {
        assert_eq!(profile_layout(1.5, 5.0), (7, 7.5, 0.2));
        assert_eq!(profile_layout(0.0, 5.0).0, 0);
        assert_eq!(profile_layout(1.0, 0.0).0, 0);
        assert_eq!(profile_layout(100.0, 100.0).0, MOST_PROFILES);
    }

    #[test]
    fn the_last_profile_can_fall_short_of_the_end() {
        let (count, profiles, _) = profile_layout(1.5, 5.0);
        assert!(percentile(count - 1, profiles) < 1.0);
        assert_eq!(percentile(0, profiles), 0.0);
        assert_eq!(percentile(0, 1.0), 0.0, "a row of one divides by nothing");
    }

    #[test]
    fn overdraw_is_every_profile_head_on_and_what_overlaps_from_the_side() {
        assert_eq!(overdraw(10.0, 0.5, 0.0), 10.0);
        assert_eq!(overdraw(10.0, 0.5, 0.1), 5.0);
        assert_eq!(overdraw(4.0, 0.5, 0.1), 4.0);
    }

    #[test]
    fn a_ratio_nothing_gave_comes_from_how_far_a_function_peaks() {
        use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
        let ranged = |low: f32, high: f32| {
            let mut function = H2Function::new(FunctionType::Identity);
            function.set_clamp_range(low, high).unwrap();
            let function = TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap();
            H3VolumeProperty { function: Arc::new(function), ..property(H3VolumeInput::Percentile, H3VolumeInput::Percentile) }
        };
        assert_eq!(ranged(0.5, 0.5).peak_over_mean(), 1.0);
        assert_eq!(ranged(0.0, 1.0).peak_over_mean(), 2.0);
    }

    fn property(input: H3VolumeInput, range: H3VolumeInput) -> H3VolumeProperty {
        H3VolumeProperty {
            input,
            range,
            adds: None,
            modifier_input: H3VolumeInput::Percentile,
            // A curve, for what allocates seeds only past a constant.
            function: Arc::new(TagFunction::parse(&[3; 32]).unwrap_or_else(|_| TagFunction::parse(&[0; 32]).unwrap())),
        }
    }

    #[test]
    fn inputs_read_by_their_names() {
        let state = H3VolumeState { age: 0.3, effect_a: 0.7, effect_b: 0.2, seeds: [0.4, 0.6], ..Default::default() };
        assert_eq!(H3VolumeInput::named("effect a scale").of(&state), 0.7);
        assert_eq!(H3VolumeInput::named("effect b scale").of(&state), 0.2);
        assert_eq!(H3VolumeInput::named("light volume age").of(&state), 0.3);
        assert_eq!(H3VolumeInput::named("light volume correlation 2").of(&state), 0.6);
    }

    #[test]
    fn a_name_no_longer_in_the_game_reads_as_invalid() {
        let state = H3VolumeState { age: 0.3, ..Default::default() };
        assert_eq!(H3VolumeInput::named("light volume lifespan"), H3VolumeInput::Invalid);
        assert_eq!(H3VolumeInput::Invalid.of(&state), 0.0);
    }

    #[test]
    fn what_runs_once_per_volume_cannot_read_a_profile_or_a_random() {
        assert_eq!(H3VolumeInput::Percentile.per_volume(), H3VolumeInput::Invalid);
        assert_eq!(H3VolumeInput::Random.per_volume(), H3VolumeInput::Invalid);
        assert_eq!(H3VolumeInput::Age.per_volume(), H3VolumeInput::Age);
        assert_eq!(H3VolumeInput::Seed(1).per_volume(), H3VolumeInput::Seed(1));
    }

    #[test]
    fn randoms_take_seeds_in_turn_after_any_a_tag_named() {
        let (mut input, mut next) = (H3VolumeInput::Random, 1);
        allocate_seed(&mut input, &mut next);
        assert_eq!(input, H3VolumeInput::Seed(1));
        let mut wrapped = H3VolumeInput::Random;
        allocate_seed(&mut wrapped, &mut next);
        assert_eq!(wrapped, H3VolumeInput::Seed(0), "two seeds, then round again");
        let named = property(H3VolumeInput::Seed(0), H3VolumeInput::Age);
        assert_eq!(first_free_seed(&[&named]), 1);
        assert_eq!(first_free_seed(&[&property(H3VolumeInput::Age, H3VolumeInput::Age)]), 0);
    }

    fn h2_function(kind: blam_tags::tag_function::FunctionType, low: f32, high: f32) -> Arc<TagFunction> {
        use blam_tags::tag_function::{FunctionEncoding, H2Function};
        let mut function = H2Function::new(kind);
        function.set_clamp_range(low, high).unwrap();
        Arc::new(TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap())
    }

    /// A constant, which reads nothing.
    fn fixed(value: f32) -> H3VolumeProperty {
        use blam_tags::tag_function::FunctionType;
        let function = h2_function(FunctionType::Constant, value, value);
        H3VolumeProperty { function, ..property(H3VolumeInput::Invalid, H3VolumeInput::Invalid) }
    }

    /// Its input, from `low` to `high`.
    fn reading(input: H3VolumeInput, low: f32, high: f32) -> H3VolumeProperty {
        use blam_tags::tag_function::FunctionType;
        H3VolumeProperty { function: h2_function(FunctionType::Identity, low, high), ..property(input, H3VolumeInput::Invalid) }
    }

    /// A row of 4 profiles a unit apart, white, alpha by `alpha`.
    fn definition(reach: bool, alpha: H3VolumeProperty) -> H3VolumeDefinition {
        H3VolumeDefinition {
            length: fixed(4.0),
            offset: fixed(0.0),
            density: fixed(1.0),
            profile_length: fixed(0.5),
            thickness: fixed(0.5),
            color: fixed(1.0),
            alpha,
            intensity: fixed(2.0),
            brightness_ratio: 1.0,
            blend: H3Blend::AddSrcTimesSrcAlpha,
            bitmap: None,
            sampler: None,
            sort_layer: 2,
            reach,
            circular: None,
            depth_fade: None,
            lod: None,
            precompiled: None,
            h4: None,
        }
    }

    fn at(definition: H3VolumeDefinition) -> H3LightVolumeAt {
        H3LightVolumeAt {
            object: 0,
            node: 0,
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            world_oriented: false,
            only_under: None,
            scale_by: String::new(),
            secondary_by: String::new(),
            seeds: [0.0; SEEDS],
            times: Arc::new(EffectTimes::read_nothing()),
            event: 0,
            definition: Arc::new(definition),
        }
    }

    /// Side on, from 10 units out.
    fn view(distance: f32) -> ViewFrame {
        ViewFrame { eye: [0.0, -distance, 0.0], forward: [0.0, 1.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0] }
    }

    /// Reach's state 6 is the LOD amount, 1 with `lod enabled` off; Halo
    /// 3's reads 0.
    #[test]
    fn reachs_lod_input_reads_the_amount_and_halo_3s_none() {
        let state = H3VolumeState { lod: 0.7, ..Default::default() };
        assert_eq!(H3VolumeInput::Lod.of(&state), 0.7);
        let alpha_of = |reach: bool| {
            let volume = at(definition(reach, reading(H3VolumeInput::Lod, 0.0, 1.0)));
            volume.quads(None, None, &view(10.0), 0.5, 0.0, 0.0).first().map(|quad| quad.color[3])
        };
        assert!(alpha_of(true).is_some_and(|alpha| alpha > 0.0), "{:?}", alpha_of(true));
        assert_eq!(alpha_of(false), Some(0.0), "Halo 3's lod is the nearest, 0");
    }

    /// `calc_lod_amount`: in from the in distance over its feather, out to
    /// the out distance over its, and nothing with `lod always 1.0`.
    #[test]
    fn a_reach_volume_fades_in_and_out_by_its_lod_distances() {
        let lod = ReachVolumeLod::postprocessed([30.0, 15.0], [100.0, 0.0], false);
        assert_eq!(lod_amount(None, 1000.0), 1.0, "no lod, always whole");
        assert_eq!(lod_amount(Some(&lod), 30.0), 0.0);
        assert_eq!(lod_amount(Some(&lod), 37.5), 0.5);
        assert_eq!(lod_amount(Some(&lod), 50.0), 1.0);
        assert_eq!(lod_amount(Some(&lod), 100.0), 0.0);
        let quirk = ReachVolumeLod { always_one: true, ..lod };
        assert_eq!(lod_amount(Some(&quirk), 50.0), 0.0, "enabled and always 1 draws nothing");
        let gated = |lod: Option<ReachVolumeLod>, reach: bool, distance: f32| {
            let volume = at(H3VolumeDefinition { lod, ..definition(reach, fixed(1.0)) });
            volume.quads(None, None, &view(distance), 0.5, 0.0, 0.0).len()
        };
        assert_eq!(gated(Some(lod), true, 10.0), 0, "nearer than its in distance");
        assert_eq!(gated(Some(lod), true, 50.0), 4);
        assert_eq!(gated(None, false, 10.0), 4, "Halo 3's has no gate");
    }

    /// The postprocess's defaults: an out distance of none is 20, the in
    /// distance at least epsilon, and each feather inverted over at least it.
    #[test]
    fn reachs_postprocess_fills_in_lod_distances_and_inverses() {
        let lod = ReachVolumeLod::postprocessed([0.0, 0.0], [0.0, 10.0], false);
        assert_eq!(lod.far, DEFAULT_LOD_OUT);
        assert_eq!(lod.near, EPSILON);
        assert_eq!(lod.inverse_in, EPSILON.recip());
        assert_eq!(lod.inverse_out, 0.1);
        assert_eq!(ReachVolumeLod::postprocessed([5.0, 2.0], [15.0, 10.0], false).far, 15.0);
    }

    /// `lights__light_volume.hlsl`: Reach's alpha is by intensity too and its
    /// colour at `V_ILLUM_EXPOSURE` and the stops; a baked row's carries its
    /// own. Halo 3's is as it was.
    #[test]
    fn a_profile_colour_takes_intensity_on_alpha_and_each_games_exposure() {
        let state = H3VolumeState::default();
        let reach = definition(true, fixed(0.5));
        let halo3 = definition(false, fixed(0.5));
        // What a scalar function makes of a colour, at intensity 2.
        let lit = reach.color.color(&state)[0] * 2.0;
        let [red, _, _, alpha] = reach.profile_color(&state, None, [1.0, 2.0], 0.0);
        assert_eq!(red, lit * reach_alt_exposure(0.0)[1]);
        assert_eq!(alpha, 0.5 * 2.0 / 2.0);
        let [brighter, ..] = reach.profile_color(&state, None, [1.0, 2.0], 1.0);
        assert_eq!(brighter, lit * reach_alt_exposure(1.0)[1]);
        assert_ne!(brighter, red, "the stops reach it");
        let [red, _, _, alpha] = halo3.profile_color(&state, None, [1.0, 2.0], 1.0);
        assert_eq!(red, lit * H3_EXPOSURE);
        assert_eq!(alpha, 0.5 * 2.0 / 2.0, "Halo 3's float4 colour scales alpha by intensity too");
        let [red, _, _, alpha] = reach.profile_color(&state, Some([3.0, 0.0, 0.0]), [1.0, 2.0], 0.0);
        assert_eq!(red, 3.0 * reach_alt_exposure(0.0)[1], "unclamped");
        assert_eq!(alpha, 0.5, "a baked row's alpha is the ratio over the overdraw");
        let multiply = H3VolumeDefinition { blend: H3Blend::Multiply, ..definition(true, fixed(0.5)) };
        assert_eq!(multiply.profile_color(&state, None, [1.0, 1.0], 1.0)[0], lit, "a multiply is not exposed");
    }

    /// `get_uses_any_of_these_states`: a constant reads nothing, a range
    /// only when ranged, a modifier only with one.
    #[test]
    fn what_a_property_reads_besides_its_place() {
        assert!(!fixed(1.0).reads_more_than_its_place());
        assert!(!reading(H3VolumeInput::Percentile, 0.0, 1.0).reads_more_than_its_place());
        assert!(reading(H3VolumeInput::Age, 0.0, 1.0).reads_more_than_its_place());
        let unranged = H3VolumeProperty { range: H3VolumeInput::Age, ..reading(H3VolumeInput::Percentile, 0.0, 1.0) };
        assert!(!unranged.reads_more_than_its_place(), "an unranged function never reads its range");
        let modified = H3VolumeProperty { adds: Some(true), modifier_input: H3VolumeInput::EffectA, ..unranged.clone() };
        assert!(modified.reads_more_than_its_place());
        let unmodified = H3VolumeProperty { adds: None, ..modified };
        assert!(!unmodified.reads_more_than_its_place());
    }

    /// `max profile count`: the most density and length can make, held to
    /// 128; and a volume bakes when what it reads is the profile's place,
    /// unless told not to.
    #[test]
    fn a_reach_volume_reading_only_its_place_precompiles_up_to_its_max() {
        assert_eq!(max_profile_count(&fixed(2.0), &fixed(3.5)), 7);
        assert_eq!(max_profile_count(&reading(H3VolumeInput::Age, 1.0, 4.0), &fixed(3.0)), 12, "a curve's high end");
        assert_eq!(max_profile_count(&fixed(100.0), &fixed(100.0)), MOST_PROFILES as usize);
        assert_eq!(max_profile_count(&fixed(-1.0), &fixed(3.0)), 0);
        let place = reading(H3VolumeInput::Percentile, 0.0, 1.0);
        let constant = fixed(1.0);
        let only_place = [&constant, &constant, &constant, &constant, &place, &place, &place, &place];
        let none = |_: &str| false;
        assert!(precompiles(none, &only_place, 4));
        assert!(!precompiles(none, &only_place, 0), "no rows to bake");
        assert!(!precompiles(|flag: &str| flag == "disable precompiled profiles", &only_place, 4));
        let age = reading(H3VolumeInput::Age, 0.0, 1.0);
        let live = [&constant, &constant, &constant, &constant, &place, &place, &age, &place];
        assert!(!precompiles(none, &live, 4));
        assert!(precompiles(|flag: &str| flag == "force precompile profiles", &live, 4));
    }

    /// The baked rows sit at `i / (max − 0.5)` and hold colour by intensity
    /// squared by alpha, and draw `max` of them on the live spacing.
    #[test]
    fn a_baked_row_is_placed_by_its_max_and_draws_that_many() {
        let mut volume = definition(true, reading(H3VolumeInput::Percentile, 0.0, 1.0));
        let rows = volume.bake(3);
        assert_eq!(rows.len(), 3);
        let alpha = |index: f32| index / 2.5;
        // Intensity 2, squared.
        let white = volume.color.color(&H3VolumeState::default())[0] * 4.0;
        for (index, (color, thickness)) in rows.iter().enumerate() {
            assert!((color[0] - white * alpha(index as f32)).abs() < 1e-5, "{index}: {color:?}");
            assert_eq!(*thickness, 0.5);
        }
        volume.precompiled = Some(rows);
        let quads = at(volume).quads(None, None, &view(10.0), 0.5, 0.0, 0.0);
        assert_eq!(quads.len(), 3, "the max, not the live 4");
        let middle = |quad: &AntennaQuad| quad.corners.iter().map(|corner| corner[0]).sum::<f32>() / 4.0;
        assert!((middle(&quads[1]) - middle(&quads[0]) - 1.0).abs() < 1e-5, "a unit apart, as live");
    }

    /// With `BLAM_TEST_HREK`: Reach's volumes draw the `circular` disc from
    /// its option's defaults with no map, fade on `on`, and a `lod enabled`
    /// one reads its distances.
    #[test]
    fn kit_reach_volumes_read_circular_depth_fade_and_lod() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = super::super::super::particles::loose_kit(&root, GameId::HaloReach);
        let first = |path: &str| {
            let tag = load_referenced_tag_from_source(&source, path, "light_volume_system", b"ltvl").expect(path);
            read_definition(&crate::app::model_preview::load::TagReader::new(&source), elements(tag.root(), "light_volumes")[0]).expect(path)
        };
        let headlights = first("objects\\vehicles\\human\\civilian\\cargo_truck\\fx\\headlights");
        assert!(headlights.reach);
        assert_eq!(headlights.circular, Some([1.1, 2.0]));
        assert!(headlights.bitmap.is_none(), "a disc samples no map");
        assert_eq!(headlights.depth_fade, Some(0.1));
        assert!(headlights.lod.is_none());
        let roof = first("levels\\solo\\m52\\fx\\roof_glow\\roof_glow");
        assert_eq!(roof.lod, Some(ReachVolumeLod::postprocessed([30.0, 15.0], [100.0, 0.0], false)));
    }

    /// With `BLAM_TEST_H3EK`: Halo 3's volumes keep their map, and none of
    /// Reach's disc, fade, lod or baked rows.
    #[test]
    fn kit_halo_3_volumes_keep_their_map() {
        let Ok(root) = std::env::var("BLAM_TEST_H3EK") else {
            eprintln!("skipping: BLAM_TEST_H3EK is not set");
            return;
        };
        let source = super::super::super::particles::loose_kit(&root, GameId::Halo3);
        let path = "objects\\characters\\masterchief\\fx\\flashlight";
        let tag = load_referenced_tag_from_source(&source, path, "light_volume_system", b"ltvl").expect(path);
        let volume = read_definition(&crate::app::model_preview::load::TagReader::new(&source), elements(tag.root(), "light_volumes")[0]).expect(path);
        assert!(!volume.reach && volume.bitmap.is_some());
        assert!(volume.circular.is_none() && volume.depth_fade.is_none() && volume.lod.is_none());
        assert!(volume.precompiled.is_none());
    }

    /// `WriteProfileMemexportedState`: the position and percentile as they
    /// are, thickness and intensity as halves, the colour saturated into
    /// ARGB8 (`PackARGB8`), which an intensity over 1 survives.
    #[test]
    fn a_profile_packs_as_the_update_writes_it() {
        let state = H4ProfileState {
            position: [1.0, -2.0, 3.5],
            percentile: 0.25,
            color: [2.0, 0.5, -1.0, 1.0],
            thickness: 0.5,
            intensity: 3.0,
        };
        let words = pack_profile_state(&state);
        assert_eq!(words[..4], [1.0f32, -2.0, 3.5, 0.25].map(f32::to_bits));
        // Half 0.5 is 0x3800, half 3 is 0x4200.
        assert_eq!(words[4], 0x3800 | (0x4200 << 16));
        // Red saturates to 255, blue to 0; green truncates 127.5 to 127.
        assert_eq!(words[5], (255 << 24) | (255 << 16) | (127 << 8));
        assert_eq!(words[6..], [0, 0]);
        let dimmer = pack_profile_state(&H4ProfileState { intensity: 1.0, ..state });
        assert_ne!(dimmer[4], words[4], "intensity rides in the halves, not the colour");
        assert_eq!(dimmer[5], words[5]);
    }

    /// Halo 4's frame: profiles placed along the direction from the offset a
    /// spacing apart, each with its own place, and the overall state the
    /// whole product of length and density, the LOD amount in state 6.
    #[test]
    fn a_halo_4_frame_places_its_profiles_and_fills_its_inputs() {
        let color = reading(H3VolumeInput::Percentile, 0.0, 1.0);
        let base = H3VolumeDefinition {
            length: fixed(3.5),
            offset: fixed(0.5),
            density: fixed(2.0),
            color,
            reach: true,
            ..definition(true, fixed(1.0))
        };
        assert!(at(definition(true, fixed(1.0))).h4_draw(None, None, &view(10.0), 0.5, 0.0).is_none(), "not Halo 4's");
        let volume = at(base);
        let draw = volume.h4_frame(3, None, None, &view(10.0), 0.5, 0.0).expect("a frame");
        // 3.5 × 2 = 7 profiles, 0.5 apart.
        assert_eq!(draw.profiles, 7.0);
        assert_eq!(draw.distance, 0.5);
        assert_eq!(draw.states.len(), 7);
        assert_eq!(draw.appearance, 3.0);
        let x = |words: &[u32; 8]| f32::from_bits(words[0]);
        assert_eq!(x(&draw.states[0]), 0.5);
        assert_eq!(x(&draw.states[6]), 0.5 + 0.5 * 6.0);
        let place = |words: &[u32; 8]| f32::from_bits(words[3]);
        assert_eq!(place(&draw.states[0]), 0.0);
        assert_eq!(place(&draw.states[6]), 1.0);
        // The colour reads the place: red rises along the row.
        let red = |words: &[u32; 8]| (words[5] >> 16) & 0xFF;
        assert!(red(&draw.states[6]) > red(&draw.states[0]));
        assert_eq!(draw.inputs[0], 0.0, "the place is the profile's");
        assert_eq!(draw.inputs[6], 1.0, "no lod enabled: the full amount");
        assert_eq!(draw.inputs[7], 1.0, "no scale named: effect a is 1");
    }

    /// With `BLAM_TEST_H4EK`: Halo 4's volumes draw through their
    /// material's `light_volume` pass and sort by its layer, with none of
    /// Reach's map, disc or baked rows.
    #[test]
    fn kit_halo_4_volumes_draw_through_their_material() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            eprintln!("skipping: BLAM_TEST_H4EK is not set");
            return;
        };
        let source = super::super::super::particles::loose_kit(&root, GameId::Halo4);
        let path = "objects\\vehicles\\covenant\\turrets\\plasma_turret\\fx\\tripod_light_volume";
        let tag = load_referenced_tag_from_source(&source, path, "light_volume_system", b"ltvl").expect(path);
        let volume = read_definition(&crate::app::model_preview::load::TagReader::new(&source), elements(tag.root(), "light_volumes")[0]).expect(path);
        let (material, _) = volume.h4.as_ref().expect("a material");
        assert!(material.pass.vertex.contains("vs_light_volume_state_buffer"));
        assert!(material.pass.vertex.contains("vs_lightVolumeOverallState"));
        assert!(volume.bitmap.is_none() && volume.circular.is_none() && volume.precompiled.is_none());
        let draw = at(volume).h4_draw(None, None, &view(2.0), 0.5, 0.0).expect("it draws at rest");
        assert!(!draw.states.is_empty());
        // Every volume in the kit reads onto its material.
        let mut failures = Vec::new();
        let mut read = 0;
        let mut stack = vec![std::path::PathBuf::from(&root)];
        while let Some(folder) = stack.pop() {
            for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "light_volume_system") {
                    continue;
                }
                let relative = path.strip_prefix(&root).unwrap().with_extension("");
                let relative = relative.to_string_lossy().replace('/', "\\");
                let tag = load_referenced_tag_from_source(&source, &relative, "light_volume_system", b"ltvl").unwrap();
                for element in elements(tag.root(), "light_volumes") {
                    match read_definition(&crate::app::model_preview::load::TagReader::new(&source), element) {
                        Some(volume) if volume.h4.is_some() => read += 1,
                        _ => failures.push(relative.clone()),
                    }
                }
            }
        }
        eprintln!("NOTE KIT {read} Halo 4 volumes read, {} failed: {failures:?}", failures.len());
        assert!(failures.is_empty());
        assert!(read > 150, "volumes read: {read}");
    }

    #[test]
    fn two_volumes_get_different_seeds_inside_the_unit_range() {
        let (first, second) = (volume_seeds(0), volume_seeds(1));
        assert_ne!(first, second);
        assert!(first.iter().chain(&second).all(|seed| (0.0..=1.0).contains(seed)));
    }
}
