//! A tracer definition: its shape, level of detail and path, as read off the tag.

use super::super::super::TagReader;
use super::properties::{
    ANGLE_FADED, EDGE_FADED, ORIGIN_FADED, PROPERTY_NAMES, RADIANS, SORT_BIAS_STEP, TracerProperty,
    UPLOADED_STATES,
};
use super::{H4EffectMaterial, has_flag, resolve_h4_effect_material};
use blam_tags::TagStruct;

/// One of a `tracer_system`'s `tracers` as `PostprocessBlockCallback`
/// (X 0x831FC270) leaves it, but for its material.
#[derive(Debug)]
pub(crate) struct TracerDefinition {
    /// `profile shape`: 0 ribbon, 1 cross, 2 n-gon, 3 horizontal, 4 vertical.
    pub(super) shape: u32,
    pub(super) ngon_sides: u32,
    /// The tag's appearance flags and the runtime fade bits.
    pub(super) appearance: u32,
    /// `1/origin range, origin cutoff, 1/edge range, edge cutoff`, the edge
    /// pair in radians.
    pub(super) fade: [f32; 4],
    /// `origin fade cutoff` and `range`, when it fades from its origin:
    /// trailing profiles are split at both ends (`Tracer::Extend`).
    pub(super) origin_fade: Option<[f32; 2]>,
    pub(super) lifespan: TracerProperty,
    pub(super) acceleration: TracerProperty,
    pub(super) length: TracerProperty,
    pub(super) offset: TracerProperty,
    pub(super) density: TracerProperty,
    pub(super) size: TracerProperty,
    pub(super) profile_offset: TracerProperty,
    pub(super) rotation: TracerProperty,
    pub(super) color: TracerProperty,
    pub(super) alpha: TracerProperty,
    pub(super) alpha2: TracerProperty,
    pub(super) black_point: TracerProperty,
    pub(super) palette: TracerProperty,
    pub(super) intensity: TracerProperty,
    pub(super) uv_tiling: [f32; 2],
    pub(super) uv_scroll: [f32; 2],
    pub(super) sort_bias: f32,
    /// `m_maxProfileCount`: none draws at 0.
    pub(super) most_profiles: i32,
}

/// A tracer and the material it draws through.
#[derive(Debug)]
pub(crate) struct H4TracerDefinition {
    tracer: TracerDefinition,
    pub(crate) material: H4EffectMaterial,
    pub(super) sort_layer: i32,
}

impl std::ops::Deref for H4TracerDefinition {
    type Target = TracerDefinition;

    fn deref(&self) -> &TracerDefinition {
        &self.tracer
    }
}

/// `tracerProfileShapeEnum`'s index.
fn shape_named(name: &str) -> u32 {
    match name {
        "cross" => 1,
        "n-gon" => 2,
        "horizontal" => 3,
        "vertical" => 4,
        _ => 0,
    }
}

pub(super) fn read_tracer(
    reader: &TagReader,
    element: TagStruct<'_>,
) -> Option<H4TracerDefinition> {
    if has_flag(element, "flags", "disabled for debugging") {
        return None;
    }
    let material =
        blam_tags::material::Material::from_struct(&element.descend("actual material?")?);
    let resolved = resolve_h4_effect_material(reader, &material, "tracer")
        .map_err(|error| eprintln!("h4 tracer material: {error}"))
        .ok()?;
    let mut properties = Vec::with_capacity(PROPERTY_NAMES.len());
    for name in PROPERTY_NAMES {
        properties.push(TracerProperty::read(element, name)?);
    }
    // The used states, then each property's seeds in turn.
    let mut used = properties
        .iter()
        .fold(0, |mask, property| mask | property.used_states())
        & UPLOADED_STATES;
    for property in &mut properties {
        property.allocate_seeds(&mut used);
    }
    let [
        lifespan,
        acceleration,
        length,
        offset,
        size,
        density,
        profile_offset,
        rotation,
        _rotation_rate,
        color,
        alpha,
        alpha2,
        black_point,
        palette,
        intensity,
    ]: [TracerProperty; 15] = properties.try_into().ok()?;
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let (origin_range, origin_cutoff) = (real("origin fade range"), real("origin fade cutoff"));
    let (edge_range, edge_cutoff) = (real("edge fade range"), real("edge fade cutoff"));
    let mut appearance = element.read_int_any("appearance flags").unwrap_or(0) as u32 & 0x3F;
    if origin_range != 0.0 || origin_cutoff != 0.0 {
        appearance |= ORIGIN_FADED;
    }
    if edge_range != 0.0 || edge_cutoff != 0.0 {
        appearance |= EDGE_FADED;
    }
    if real("angle fade range") != 0.0 || real("angle fade begin") != 0.0 {
        appearance |= ANGLE_FADED;
    }
    let vector = |name: &str| {
        let vector = element.read_vec2(name);
        [vector.i, vector.j]
    };
    // `uv tiling` of 0 is 1, each way.
    let uv_tiling = vector("uv tiling").map(|tiling| if tiling == 0.0 { 1.0 } else { tiling });
    let most_profiles = (length.most() * density.most()) as i32;
    let tracer = TracerDefinition {
        shape: shape_named(
            &element
                .read_enum_option("profile shape")
                .unwrap_or_default(),
        ),
        ngon_sides: element
            .read_int_any("number of n-gon sides")
            .unwrap_or(0)
            .max(0) as u32,
        appearance,
        fade: [
            origin_range.recip(),
            origin_cutoff,
            (edge_range * RADIANS).recip(),
            edge_cutoff * RADIANS,
        ],
        origin_fade: (appearance & ORIGIN_FADED != 0).then_some([origin_cutoff, origin_range]),
        lifespan,
        acceleration,
        length,
        offset,
        density,
        size,
        profile_offset,
        rotation,
        color,
        alpha,
        alpha2,
        black_point,
        palette,
        intensity,
        uv_tiling,
        uv_scroll: vector("uv scrolling"),
        sort_bias: element.read_int_any("sort bias").unwrap_or(0) as f32 * SORT_BIAS_STEP,
        most_profiles,
    };
    Some(H4TracerDefinition {
        tracer,
        sort_layer: i32::from(material.sort_layer).clamp(1, 3),
        material: resolved,
    })
}

impl TracerDefinition {
    /// Ribbons, horizontals and verticals are one strip, a cross two, an
    /// n-gon one a side.
    pub(super) fn strips(&self) -> u32 {
        match self.shape {
            1 => 2,
            2 => self.ngon_sides,
            _ => 1,
        }
    }
}

/// A system's `far lod` pair and the values at each end, as the postprocess
/// leaves them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TracerLod {
    pub(super) start: f32,
    pub(super) end: f32,
    pub(super) start_value: f32,
    pub(super) end_value: f32,
}

impl TracerLod {
    /// `PostprocessCallback`: with neither value set, 1 near and 0 far.
    pub(super) fn read(root: TagStruct<'_>) -> Self {
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let (mut start_value, mut end_value) =
            (real("far lod start value"), real("far lod end value"));
        if start_value < 1e-4 && end_value < 1e-4 {
            (start_value, end_value) = (1.0, 0.0);
        }
        Self {
            start: real("far lod start"),
            end: real("far lod end"),
            start_value,
            end_value,
        }
    }

    /// `ComputeLod` (X 0x834DA128) at `distance`, the camera's scale taken
    /// as 1 \[open\].
    pub(super) fn at(&self, distance: f32) -> f32 {
        let span = self.end - self.start;
        let t = if span == 0.0 {
            0.0
        } else {
            ((distance - self.start) / span).clamp(0.0, 1.0)
        };
        self.start_value + t * (self.end_value - self.start_value)
    }
}

/// Where a tracer runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum TracerPath {
    /// From its location to the part's secondary location.
    PointToPoint { end: (usize, [f32; 3]) },
    /// Behind its location as it moves, its forward the marker's.
    Trailing { forward: [f32; 3] },
}
