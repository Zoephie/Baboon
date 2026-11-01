//! Halo 3's light volumes: a row of camera-facing profiles along a direction.
//! The game works these out on the card every frame, but nothing in one
//! depends on the last, so they are worked out here a frame at a time.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::h3_effects::adopt_event_function;
use super::ce_attachments::elements;
use super::ce_lights::{CeAntennaQuad, CeFlareView, posed_on};
use super::h2_functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::{
    H3EffectTimes, for_each_attached_part, part_frame, part_markers, read_part_placement,
};
use super::h3_shaders::{H3Blend, read_h3_definition};
use super::materials::{H3Bitmap, read_h3_bitmap};
use super::renderer::H3_EXPOSURE;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// What a volume's row can hold, whatever its tag says.
const MOST_PROFILES: i32 = 128;
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
            // The nearest of a preview's levels of detail.
            Self::Random | Self::Lod | Self::Invalid => 0.0,
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
        let input = |field: &str| H3VolumeInput::named(&property.read_enum_name(field).unwrap_or_default());
        let adds = match property.read_enum_name("Output Modifier").as_deref() {
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

    fn read_per_volume(holder: TagStruct<'_>, name: &str) -> Option<Self> {
        let mut property = Self::read(holder, name)?;
        property.input = property.input.per_volume();
        property.range = property.range.per_volume();
        property.modifier_input = property.modifier_input.per_volume();
        Some(property)
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
    let step = |state: u32| state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let first = step(index as u32);
    [first, step(first)].map(|state| (state >> 16) as f32 / 65_535.0)
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
    bitmap: H3Bitmap,
    sort_layer: i32,
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
    times: Arc<H3EffectTimes>,
    event: usize,
    pub(crate) definition: Arc<H3VolumeDefinition>,
}

fn read_definition(source: &TagSource, element: TagStruct<'_>) -> Option<H3VolumeDefinition> {
    let shader = element.descend("actual shader?")?;
    let parameters = elements(shader, "parameters");
    let base = parameters.iter().find(|parameter| {
        parameter.read_string_id("parameter name").as_deref() == Some("base_map")
    })?;
    let map = base.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
    let definition = shader.read_tag_ref_path("definition").filter(|path| !path.is_empty())?;
    let definition = read_h3_definition(source, &definition)?;
    let options: Vec<usize> = elements(shader, "options")
        .iter()
        .map(|option| option.read_int_any("short").unwrap_or(0).max(0) as usize)
        .collect();
    let once = |name: &str| H3VolumeProperty::read_per_volume(element, name);
    let each = |name: &str| H3VolumeProperty::read(element, name);
    let (length, offset, density, profile_length) =
        (once("length")?, once("offset")?, once("profile_density")?, once("profile_length")?);
    let (mut thickness, mut color, mut alpha, mut intensity) = (
        each("profile_thickness")?,
        each("profile_color")?,
        each("profile_alpha")?,
        each("profile_intensity")?,
    );
    let mut next = first_free_seed(&[&length, &offset, &density, &profile_length, &thickness, &color, &alpha, &intensity]);
    for property in [&mut thickness, &mut color, &mut alpha, &mut intensity] {
        property.allocate_seeds(&mut next);
    }
    Some(H3VolumeDefinition {
        length,
        offset,
        density,
        profile_length,
        thickness,
        color,
        alpha,
        intensity,
        brightness_ratio: element.read_real("brightness ratio").unwrap_or(1.0),
        blend: definition.blend(&options),
        bitmap: read_h3_bitmap(source, &map, 0, SamplerKind::Texture2D)?,
        sort_layer: super::h3_particles::h3_sort_layer(shader),
    })
}

/// The volumes an object's attached effects run, one at each marker of a
/// location.
pub(super) fn read_h3_light_volumes(
    source: &TagSource,
    object: &blam_tags::TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3LightVolumeAt> {
    let mut volumes = Vec::new();
    let volume_group = u32::from_be_bytes(*b"ltvl");
    for_each_attached_part(source, object, |part| {
        if part.group != volume_group {
            return;
        }
        let Ok(tag) = load_referenced_tag_from_source(source, &part.path, "light_volume_system", b"ltvl") else {
            return;
        };
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for element in elements(tag.root(), "light_volumes") {
            let Some(definition) = read_definition(source, element).map(Arc::new) else {
                continue;
            };
            for marker in part_markers(markers, &part.marker, part.effect.once) {
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

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length_of(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = length_of(a).max(1e-6);
    a.map(|axis| axis / length)
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
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    /// Follow the regions `merge_attached` renames for what `name` brings.
    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        adopt_event_function(name, &mut self.scale_by, &mut self.object);
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

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

    pub(crate) fn bitmap(&self) -> &H3Bitmap {
        &self.definition.bitmap
    }

    pub(crate) fn sort_layer(&self) -> i32 {
        self.definition.sort_layer
    }

    pub(crate) fn blend(&self) -> H3Blend {
        self.definition.blend
    }

    /// The states this frame, or none while its event is not running.
    fn state(&self, values: Option<&H2FunctionValues>, time: f32, onset: f32) -> Option<H3VolumeState> {
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
            effect_a,
            effect_b: read(&self.secondary_by, 1.0),
        })
    }

    /// Each profile's quad this frame, nearest the marker first.
    pub(crate) fn quads(
        &self,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
        onset: f32,
    ) -> Vec<CeAntennaQuad> {
        let Some(mut state) = self.state(values, time, onset) else {
            return Vec::new();
        };
        let volume = &*self.definition;
        let length = volume.length.scalar(&state);
        let density = volume.density.scalar(&state);
        let (count, profiles, distance) = profile_layout(length, density);
        let (origin, turned) = posed_on(self.node, self.position, self.forward, bones);
        let direction = if self.world_oriented { self.forward } else { turned };
        let direction = unit(direction);
        let offset = volume.offset.scalar(&state);
        let side_length = volume.profile_length.scalar(&state);
        let mut quads = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            state.percentile = percentile(index, profiles);
            let along = offset + distance * index as f32;
            let place: [f32; 3] = std::array::from_fn(|axis| origin[axis] + direction[axis] * along);
            let thickness = volume.thickness.scalar(&state);
            let to_profile = unit(std::array::from_fn(|axis| place[axis] - view.eye[axis]));
            let sine = length_of(cross(direction, to_profile)).min(1.0);
            let drawn_length = thickness + (side_length - thickness) * sine;
            let up = unit(cross(to_profile, direction));
            let across = cross(to_profile, up);
            let corner = |u: f32, v: f32| {
                let (x, y) = ((u - 0.5) * drawn_length, (v - 0.5) * thickness);
                let [a, b, c] = std::array::from_fn(|axis| place[axis] + across[axis] * x + up[axis] * y);
                [a, b, c, u, v]
            };
            let color = volume.color.color(&state).map(|channel| channel.clamp(0.0, 1.0));
            let alpha = volume.alpha.scalar(&state).clamp(0.0, 1.0);
            let intensity = volume.intensity.scalar(&state);
            let ratio = volume.brightness_ratio + (1.0 - volume.brightness_ratio) * sine;
            let [red, green, blue] = color.map(|channel| channel * intensity * H3_EXPOSURE);
            quads.push(CeAntennaQuad {
                corners: [corner(0.0, 0.0), corner(1.0, 0.0), corner(0.0, 1.0), corner(1.0, 1.0)],
                color: [red, green, blue, alpha * ratio / overdraw(profiles, drawn_length, distance * sine)],
            });
        }
        quads
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn two_volumes_get_different_seeds_inside_the_unit_range() {
        let (first, second) = (volume_seeds(0), volume_seeds(1));
        assert_ne!(first, second);
        assert!(first.iter().chain(&second).all(|seed| (0.0..=1.0).contains(seed)));
    }
}
