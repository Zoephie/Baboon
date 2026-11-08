//! Reading a Halo CE particle system (`pctl`): its particle types, states and physics, and where it hangs.

use super::*;

/// A particle type's `flags`, by name. Each kind of state's
/// `... forward/backward` shares its name with the other's, so each is the
/// option after its loop.
#[derive(Debug, Clone, Copy)]
pub(super) struct ParticleTypeFlags {
    pub(super) type_states_loop: bool,
    pub(super) type_states_bounce: bool,
    pub(super) particle_states_loop: bool,
    pub(super) particle_states_bounce: bool,
    pub(super) die_in_air: bool,
    pub(super) die_on_ground: bool,
    pub(super) rotational_sideways: bool,
    pub(super) disabled: bool,
    pub(super) tint_by_effect_color: bool,
    pub(super) initial_count_scales: bool,
    pub(super) minimum_count_scales: bool,
    pub(super) creation_rate_scales: bool,
    pub(super) scale_scales: bool,
    pub(super) animation_rate_scales: bool,
    pub(super) rotation_rate_scales: bool,
}

impl ParticleTypeFlags {
    fn read(entry: TagStruct<'_>) -> Self {
        let set = |option: &str| has_flag(entry, "flags", option);
        Self {
            type_states_loop: set("type states loop"),
            type_states_bounce: has_flag_after(entry, "flags", "type states loop"),
            particle_states_loop: set("particle states loop"),
            particle_states_bounce: has_flag_after(entry, "flags", "particle states loop"),
            die_in_air: set("particles die in air"),
            die_on_ground: set("particles die on ground"),
            rotational_sideways: set("rotational sprites animate sideways"),
            disabled: set("disabled"),
            tint_by_effect_color: set("tint by effect color"),
            initial_count_scales: set("initial count scales with effect"),
            minimum_count_scales: set("minimum count scales with effect"),
            creation_rate_scales: set("creation rate scales with effect"),
            scale_scales: set("scale scales with effect"),
            animation_rate_scales: set("animation rate scales with effect"),
            rotation_rate_scales: set("rotation rate scales with effect"),
        }
    }
}

/// `particle creation physics`: where a new particle starts and how fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CreationPhysics {
    Default,
    Explosion,
    Jet,
}

impl CreationPhysics {
    fn read(holder: TagStruct<'_>) -> Self {
        match holder.read_enum_option("particle creation physics").as_deref() {
            Some("explosion") => Self::Explosion,
            Some("jet") => Self::Jet,
            _ => Self::Default,
        }
    }
}

/// A type state's multipliers, as `particle_system_type_state_interpolated_variables`
/// keeps them: scale, animation rate, rotation rate, color (alpha, red,
/// green, blue), radius, minimum count, creation rate.
pub(super) type TypeVariables = [f32; 10];

pub(super) const SCALE: usize = 0;

pub(super) const ANIMATION: usize = 1;

pub(super) const ROTATION: usize = 2;

pub(super) const COLOR: usize = 3;

pub(super) const RADIUS: usize = 7;

pub(super) const MINIMUM: usize = 8;

pub(super) const RATE: usize = 9;

/// A particle's own: scale, animation rate, rotation rate, color (alpha,
/// red, green, blue).
pub(super) type ParticleVariables = [f32; 7];

#[derive(Debug, Clone)]
pub(super) struct ParticleType {
    pub(super) flags: ParticleTypeFlags,
    pub(super) initial_count: i32,
    /// `complex sprite render modes` is `rotational`, not `simple`.
    pub(super) rotational: bool,
    pub(super) radius: f32,
    pub(super) initial_physics: CreationPhysics,
    pub(super) constants: [f32; 3],
    pub(super) states: Vec<TypeState>,
    pub(super) particle_states: Vec<ParticleState>,
}

/// A `particle_system` tag.
#[derive(Debug, Clone)]
pub(crate) struct CePctlDef {
    pub(super) types: Vec<ParticleType>,
    /// `point physics`, which moves a system on no object.
    pub(super) physics: Option<CePointPhysics>,
    /// Every bitmap its particle states name, for the textures.
    pub(crate) sheets: Vec<Arc<CeSpriteSheet>>,
}

/// A particle system on one of the object's attachments.
#[derive(Debug, Clone)]
pub(crate) struct CeParticleSystemAt {
    /// Which object's functions and change colors drive it; 0 is the opened.
    pub(crate) object: u8,
    pub(crate) def: Arc<CePctlDef>,
    /// The attachment's marker, each place it is on the model.
    pub(super) markers: Vec<(usize, [f32; 3], [f32; 3])>,
    pub(super) primary: Option<usize>,
    pub(super) change_color: Option<usize>,
}

impl super::super::functions::ReadsOutputs for CeParticleSystemAt {
    fn object(&self) -> u8 {
        self.object
    }
    fn outputs_read(&self) -> Vec<usize> {
        self.primary.into_iter().collect()
    }
}

fn bounds_of(holder: TagStruct<'_>, name: &str) -> [f32; 2] {
    match holder.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealBounds(bounds) | TagFieldData::AngleBounds(bounds) | TagFieldData::FractionBounds(bounds)) => {
            [bounds.lower, bounds.upper]
        }
        _ => [0.0; 2],
    }
}

fn argb_of(holder: TagStruct<'_>, name: &str) -> [f32; 4] {
    let [red, green, blue, alpha] = crate::app::model_preview::common::fields::argb(holder, name);
    [alpha, red, green, blue]
}

fn constants_of(holder: TagStruct<'_>) -> [f32; 3] {
    let found = elements(holder, "physics constants");
    std::array::from_fn(|at| found.get(at).and_then(|constant| constant.read_real("k")).unwrap_or(0.0))
}

fn read_physics(reader: &TagReader, holder: TagStruct<'_>) -> Option<CePointPhysics> {
    let path = holder.read_tag_ref_path("point physics").filter(|path| !path.is_empty())?;
    let tag = reader.load(&path, "point_physics", b"pphy").ok()?;
    Some(CePointPhysics::read(&tag))
}

pub(in crate::app::model_preview) fn read_pctl_def(reader: &TagReader, path: &str) -> Option<CePctlDef> {
    let tag = reader.load(path, "particle_system", b"pctl").ok()?;
    let root = tag.root();
    let mut sheets: Vec<Arc<CeSpriteSheet>> = Vec::new();
    let mut sheet_paths: Vec<String> = Vec::new();
    let mut types = Vec::new();
    for entry in elements(root, "particle types") {
        let int = |name: &str| entry.read_int_any(name).unwrap_or(0);
        let states = elements(entry, "states")
            .iter()
            .map(|state| {
                let real = |name: &str| state.read_real(name).unwrap_or(0.0);
                // Tool's `sub_49BFC0`.
                let radius = Some(real("radius multiplier")).filter(|radius| *radius > 0.0).unwrap_or(1.0);
                let [red, green, blue, alpha] = crate::app::model_preview::common::fields::argb(*state, "color multiplier");
                TypeState {
                    duration: bounds_of(*state, "duration bounds"),
                    transition: bounds_of(*state, "transition time bounds"),
                    variables: [
                        real("scale multiplier"),
                        real("animation_rate_multiplier"),
                        real("rotation rate multiplier"),
                        alpha,
                        red,
                        green,
                        blue,
                        radius,
                        real("minimum particle count"),
                        real("particle creation rate"),
                    ],
                    creation_physics: CreationPhysics::read(*state),
                }
            })
            .collect();
        let mut particle_states = Vec::new();
        for state in elements(entry, "particle states") {
            let Some(bitmap) = state.read_tag_ref_path("bitmaps").filter(|path| !path.is_empty()) else {
                continue;
            };
            let sheet_index = match sheet_paths.iter().position(|known| *known == bitmap) {
                Some(at) => at,
                None => {
                    let Some(sheet) = read_sprite_sheet(reader, &bitmap) else {
                        continue;
                    };
                    sheets.push(Arc::new(sheet));
                    sheet_paths.push(bitmap);
                    sheets.len() - 1
                }
            };
            let real = |name: &str| state.read_real(name).unwrap_or(0.0);
            particle_states.push(ParticleState {
                duration: bounds_of(state, "duration bounds"),
                transition: bounds_of(state, "transition time bounds"),
                sheet: sheets[sheet_index].clone(),
                sheet_index,
                sequence: state.read_int_any("sequence index").unwrap_or(0) as i32,
                scale: bounds_of(state, "scale"),
                animation_rate: bounds_of(state, "animation rate"),
                rotation_rate: bounds_of(state, "rotation rate"),
                colors: [argb_of(state, "color 1"), argb_of(state, "color 2")],
                // Tool's `sub_49BFE0`.
                radius: Some(real("radius multiplier")).filter(|radius| *radius > 0.0).unwrap_or(1.0),
                physics: read_physics(reader, state),
                blend: FramebufferBlend::read(state),
                map_flags: state.read_int_any("map flags").unwrap_or(0),
                nonlinear_tint: has_flag(state, "shader flags", "nonlinear tint"),
                sort_bias: has_flag(state, "shader flags", "sort bias"),
                secondary: read_secondary_map(reader, state, "bitmap"),
            });
        }
        types.push(ParticleType {
            flags: ParticleTypeFlags::read(entry),
            initial_count: int("initial particle count") as i32,
            rotational: entry.field("complex sprite render modes").is_some_and(|field| field.enum_is("rotational")),
            radius: entry.read_real("radius").unwrap_or(0.0),
            initial_physics: CreationPhysics::read(entry),
            constants: constants_of(entry),
            states,
            particle_states,
        });
    }
    // `particle_system_initialize`: every type needs a state, and one at least.
    if types.is_empty() || types.iter().any(|kind| kind.states.is_empty()) {
        return None;
    }
    Some(CePctlDef { types, physics: read_physics(reader, root), sheets })
}

/// The particle systems among the object's attachments.
pub(in crate::app::model_preview) fn read_ce_particle_systems(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeParticleSystemAt> {
    let Some(block) = object_block(object, "attachments") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"pctl");
    let mut systems = Vec::new();
    for attachment in elements(block, "attachments") {
        let Some((kind, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if kind != group || path.is_empty() {
            continue;
        }
        let Some(def) = read_pctl_def(reader, &path) else {
            continue;
        };
        let name = attachment.read_string("marker").unwrap_or_default();
        // `object_get_marker_by_name`: no name is the origin.
        let found: Vec<(usize, [f32; 3], [f32; 3])> = if name.is_empty() {
            vec![(0, [0.0; 3], [1.0, 0.0, 0.0])]
        } else {
            markers
                .iter()
                .filter(|marker| marker.name.eq_ignore_ascii_case(&name))
                .take(MOST_MARKERS)
                .map(|marker| (marker.node_index.max(0) as usize, marker.position, marker.axes[0]))
                .collect()
        };
        systems.push(CeParticleSystemAt {
            object: owner,
            def: Arc::new(def),
            markers: found,
            primary: function_slot_of(attachment, "primary scale"),
            change_color: function_slot_of(attachment, "change color"),
        });
    }
    systems
}
