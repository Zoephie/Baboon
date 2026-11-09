//! A particle's look: billboard, albedo and lighting, its frames and mesh, and how it is read.

use super::*;

/// `c_particle_definition`'s billboard styles, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum Billboard {
    ScreenFacing,
    CameraFacing,
    ScreenParallel,
    ScreenPerpendicular,
    ScreenVertical,
    ScreenHorizontal,
    LocalVertical,
    LocalHorizontal,
    World,
    VelocityHorizontal,
}

impl Billboard {
    pub(in crate::app::model_preview) fn named(name: &str) -> Self {
        match name {
            "camera facing" => Self::CameraFacing,
            "parallel to direction" => Self::ScreenParallel,
            "perpendicular to direction" => Self::ScreenPerpendicular,
            "vertical" => Self::ScreenVertical,
            "horizontal" => Self::ScreenHorizontal,
            "local vertical" => Self::LocalVertical,
            "local horizontal" => Self::LocalHorizontal,
            "world (particle models)" => Self::World,
            "velocity horizontal (particle models)" => Self::VelocityHorizontal,
            _ => Self::ScreenFacing,
        }
    }
}

/// How `sample_diffuse` reads the maps, by the `albedo` option's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParticleAlbedo {
    Diffuse = 0,
    DiffuseBillboardAlpha = 1,
    DiffuseSpriteAlpha = 2,
    Palette = 3,
    PaletteBillboardAlpha = 4,
    PaletteSpriteAlpha = 5,
    /// ODST's: tinted, less so the brighter the texel.
    DiffuseModulated = 6,
    /// ODST's: a ramp lit by the particle's palette value.
    PaletteGlow = 7,
    /// ODST's: two noises' difference, looked up in the palette.
    PalettePlasma = 8,
    /// Halo 2's `effect_shader_*_tint`: the texel's fourth power toward itself
    /// by the tint, so that bright stays white.
    H2Tint = 9,
    /// Halo 2's `effect_shader_alpha_multiply_add`, whose colour and alpha part.
    H2MultiplyAdd = 10,
    /// Halo 2's `effect_shader_alphablend_distortion`: the frame as it stands,
    /// read where its `v8u8` bump map pushes, times the particle's colour.
    H2Distortion = 11,
    /// Reach's `palettized_2d_plasma`: the plasma's noises with no alpha
    /// term, looked up at the depth fade rather than the palette value.
    Palette2dPlasma = 12,
}

/// What ODST's three new albedo options read.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct AlbedoConstants {
    pub(crate) tint: [f32; 3],
    pub(crate) modulation: f32,
    pub(crate) alpha_modulation: f32,
    pub(crate) base_xform: [f32; 4],
    pub(crate) second_xform: [f32; 4],
}

impl ParticleAlbedo {
    pub(super) fn named(name: &str) -> Self {
        match name {
            "diffuse_plus_billboard_alpha" => Self::DiffuseBillboardAlpha,
            "diffuse_plus_sprite_alpha" => Self::DiffuseSpriteAlpha,
            "palettized" => Self::Palette,
            "palettized_plus_billboard_alpha" => Self::PaletteBillboardAlpha,
            "palettized_plus_sprite_alpha" | "palettized_plus_alpha" => Self::PaletteSpriteAlpha,
            "diffuse_modulated" => Self::DiffuseModulated,
            "palettized_glow" => Self::PaletteGlow,
            "palettized_plasma" => Self::PalettePlasma,
            "palettized_2d_plasma" => Self::Palette2dPlasma,
            _ => Self::Diffuse,
        }
    }
}

/// What a game reads of a particle it draws through its own material, in
/// place of quads: Halo 4's.
pub(crate) trait ParticleMaterial: std::fmt::Debug + Send + Sync {
    /// The material as itself, for its game's draw.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A game's reader of a particle's own material, from its tag's root and
/// its model's strip.
pub(in crate::app::model_preview) type ParticleMaterialReader =
    fn(&TagReader, TagStruct<'_>, Option<&blam_tags::ParticleModelStrip>) -> Result<Arc<dyn ParticleMaterial>, String>;

/// A `.particle`.
#[derive(Debug)]
pub(crate) struct ParticleLook {
    /// `dies at rest`.
    pub(in crate::app::model_preview) dies_at_rest: bool,
    /// Whether a speed of none is nudged to a little.
    pub(in crate::app::model_preview) still_moves: bool,
    pub(in crate::app::model_preview) billboard: Billboard,
    pub(in crate::app::model_preview) flip_u: bool,
    pub(in crate::app::model_preview) flip_v: bool,
    pub(in crate::app::model_preview) random_rotation: bool,
    pub(in crate::app::model_preview) intensity_fades: bool,
    pub(in crate::app::model_preview) one_shot: bool,
    pub(in crate::app::model_preview) backwards: bool,
    /// `attachments`: effects its particles start, by what starts them.
    pub(in crate::app::model_preview) attachments: Vec<(ParticleTrigger, String)>,
    /// Whether the tag lists any attachment at all, which is what keeps its
    /// emitters off the card (`c_particle_definition::postprocess`).
    pub(in crate::app::model_preview) any_attachments: bool,
    /// A Halo 2 particle with `attached particle systems`: its own tag.
    pub(in crate::app::model_preview) carried_systems: Option<String>,
    /// Halo 2: `sub_61AE40` scales a death or collision effect by `speed - 0.5`.
    pub(in crate::app::model_preview) death_scaled_by_speed: bool,
    /// `s_mesh_variant_list`: what `Model` draws in place of a quad.
    pub(in crate::app::model_preview) model: Option<Vec<Vec<blam_tags::ParticleVertex>>>,
    /// `bitmap authored vertically`: a quarter turn, for what is not a mesh.
    pub(in crate::app::model_preview) vertical: bool,
    /// `dies on structure collision`.
    pub(in crate::app::model_preview) dies_on_ground: bool,
    /// `tint from lightmap`.
    pub(in crate::app::model_preview) tint_from_lightmap: bool,
    pub(in crate::app::model_preview) double_sided: bool,
    /// `fade when viewed edge-on`: `1 / range` and the cutoff, in radians.
    pub(in crate::app::model_preview) edge_fade: Option<[f32; 2]>,
    /// `motion blur`'s stretch.
    pub(in crate::app::model_preview) blur_aspect: Option<f32>,
    /// The postprocess's: at 1, a hemisphere.
    pub(in crate::app::model_preview) curvature: f32,
    /// `m_sprite`: the quad's corner and size in its own units.
    pub(in crate::app::model_preview) corner: [f32; 4],
    /// `m_frames`: left, top, width and height.
    pub(in crate::app::model_preview) frames: Vec<[f32; 4]>,
    /// Halo 2's `select random sequence`: each sequence's frames and corner,
    /// one of which a particle takes at birth. Empty for one sequence.
    pub(in crate::app::model_preview) sequences: Vec<(Vec<[f32; 4]>, [f32; 4])>,
    pub(in crate::app::model_preview) aspect: ParticleProperty,
    pub(in crate::app::model_preview) color: ParticleProperty,
    pub(in crate::app::model_preview) intensity: ParticleProperty,
    pub(in crate::app::model_preview) alpha: ParticleProperty,
    pub(in crate::app::model_preview) frame: ParticleProperty,
    pub(in crate::app::model_preview) rate: ParticleProperty,
    pub(in crate::app::model_preview) palette: ParticleProperty,
    pub(crate) shader: Arc<SpriteShader>,
    /// Halo 4's: drawn through its material from packed state, not as quads.
    pub(crate) h4: Option<Arc<dyn ParticleMaterial>>,
    /// Halo 3's colours are light, and Halo 2's are what shows.
    pub(in crate::app::model_preview) exposed: bool,
    /// Reach's and Halo 4's own game time, lighting, exposure and shapes:
    /// [`ParticleRules::reach_look`].
    pub(in crate::app::model_preview) reach: bool,
    /// Reach's `uses cheap shader`: `particle_render_fast.hlsl`.
    pub(in crate::app::model_preview) cheap: bool,
    /// Reach's `runtime m_used_particle_states` (prt3+460): the states its
    /// properties read, seeds allocated included, which its emitters' seeds
    /// must go around. None for Halo 3's.
    pub(in crate::app::model_preview) used_states: u32,
}

/// The `lighting` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum ParticleLighting {
    None,
    Ambient,
    Directional,
    /// Reach's `per_vertex_ambient`: the probe's ambient, on the vertex.
    ReachAmbient,
    /// Reach's `per_pixel_smooth`: between the probe's dark and bright, by
    /// how the bent normal faces its bright direction.
    ReachSmooth,
}

/// `ravi_order_0` and `ravi_order_3`, of `calculate_and_set_ravi_constants`'
/// ten: the ambient, or what comes from `normal`.
pub(super) fn ravi(lighting: ParticleLighting, normal: [f32; 3], constants: &[[f32; 4]; 10]) -> [f32; 3] {
    const C1: f32 = 0.429_043;
    const C2: f32 = 0.511_664;
    const C4: f32 = 0.886_227;
    let product = |a: [f32; 3], b: [f32; 4]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let [x, y, z] = normal;
    std::array::from_fn(|channel| {
        let ambient = C4 * constants[0][channel];
        let value = if lighting == ParticleLighting::Ambient {
            ambient
        } else {
            let linear = product(normal, constants[1 + channel]);
            let first = product([x * y, y * z, z * x], constants[4 + channel]);
            let [a, b, c, d] = constants[7 + channel];
            let second = x * x * a + y * y * b + z * z * c + d / 3.0;
            ambient - 2.0 * C2 * linear - 2.0 * C1 * first - C1 * second
        };
        value / std::f32::consts::PI
    })
}

/// `postprocess_frame_animation`'s `m_sprite`, from a sprite's left, right, top
/// and bottom, its registration point, and the particle's `center offset`.
pub(in crate::app::model_preview) fn sprite_corner(rect: [f32; 4], center: [f32; 2], offset: [f32; 2], plate_aspect: f32) -> [f32; 4] {
    let [left, right, top, bottom] = rect;
    let (width, height) = (right - left, bottom - top);
    let (mut scale_x, mut scale_y) = (1.0, 1.0);
    if width <= height {
        scale_x = width / height;
    } else {
        scale_y = height / width;
    }
    scale_x *= plate_aspect;
    let shift_x = 2.0 * (center[0] + offset[0]) / width - 1.0;
    let shift_y = 2.0 * (center[1] - offset[1]) / height - 1.0;
    let (x, y) = ((-1.0 - shift_x) * scale_x, (-1.0 - shift_y) * scale_y);
    [x, y, (1.0 - shift_x) * scale_x - x, (1.0 - shift_y) * scale_y - y]
}

/// `postprocess_frame_animation`: the quad from the first sequence's sprite at
/// the starting frame, and the frames from `first + 1`'s, manual ones first.
fn read_frames(
    reader: &TagReader,
    shader: TagStruct<'_>,
    first: usize,
    offset: [f32; 2],
    start: f32,
    one_shot: bool,
) -> (Vec<[f32; 4]>, [f32; 4]) {
    let whole = vec![[0.0, 0.0, 1.0, 1.0]];
    let tag = parameter(shader, "base_map")
        .and_then(|base| base.read_tag_ref_path("bitmap"))
        .and_then(|path| reader.load(&path, "bitmap", b"bitm").ok());
    let Some(tag) = tag else {
        return (whole, sprite_corner([0.0, 1.0, 0.0, 1.0], [0.5, 0.5], offset, 1.0));
    };
    let plate_aspect = elements(tag.root(), "bitmaps").first().map_or(1.0, |image| {
        let size = |name: &str| image.read_int_any(name).unwrap_or(1) as f32;
        size("width") / size("height")
    });
    let read = |sprite: &TagStruct<'_>, name: &str| sprite.read_real(name).unwrap_or(0.0);
    let sequences = elements(tag.root(), "sequences");
    let (mut rect, mut center) = ([0.0, 1.0, 0.0, 1.0], [0.5, 0.5]);
    if let Some(sequence) = sequences.get(first % sequences.len().max(1)) {
        let sprites = elements(*sequence, "sprites");
        let count = if one_shot { sprites.len().saturating_sub(1) } else { sprites.len() };
        if count > 0 {
            let sprite = sprites[(count as f32 * start).floor() as i32 as usize % sprites.len()];
            rect = ["left", "right", "top", "bottom"].map(|name| read(&sprite, name));
            let registration = sprite.read_point2d("registration point");
            center = [registration.x, registration.y];
        }
    }
    let manual = elements(tag.root(), "manual_sequences");
    let listed = if manual.is_empty() { sequences } else { manual };
    let frames: Vec<[f32; 4]> = listed
        .get((first + 1) % listed.len().max(1))
        .map(|sequence| elements(*sequence, "sprites"))
        .unwrap_or_default()
        .iter()
        .take(MOST_FRAMES)
        .map(|sprite| {
            let [left, right, top, bottom] = ["left", "right", "top", "bottom"].map(|name| read(sprite, name));
            [left, top, right - left, bottom - top]
        })
        .collect();
    let frames = if frames.is_empty() {
        vec![[rect[0], rect[2], rect[1] - rect[0], rect[3] - rect[2]]]
    } else {
        frames
    };
    (frames, sprite_corner(rect, center, offset, plate_aspect))
}

/// A `particle_model`'s meshes, each a triangle list.
fn read_particle_mesh(reader: &TagReader, path: &str) -> Option<Vec<Vec<blam_tags::ParticleVertex>>> {
    let tag = reader.load(path, "particle_model", b"pmdf").ok()?;
    let meshes = blam_tags::particle_model_meshes(&tag, "particle_model").ok()?;
    let triangles = |mesh: &blam_tags::ParticleObjectMesh| {
        mesh.indices.iter().filter_map(|index| mesh.vertices.get(*index as usize).copied()).collect::<Vec<_>>()
    };
    let meshes: Vec<_> = meshes.iter().map(triangles).filter(|mesh: &Vec<_>| !mesh.is_empty()).collect();
    (!meshes.is_empty()).then_some(meshes)
}

pub(super) fn read_look(reader: &TagReader, path: &str) -> Option<ParticleLook> {
    let tag = reader.load(path, "particle", b"prt3").ok()?;
    let root = tag.root();
    let model = match root.read_tag_ref_path("Model").filter(|model| !model.is_empty()) {
        Some(model) => Some(read_particle_mesh(reader, &model)?),
        None => None,
    };
    let rules = ParticleRules::of(reader.source.game());
    let h4 = match rules.read_material {
        Some(read_material) => {
            // The GPU draws a model's strip whole, variant by variant.
            let strip = match root.read_tag_ref_path("Model").filter(|model| !model.is_empty()) {
                Some(path) => {
                    let tag = reader.load(&path, "particle_model", b"pmdf").ok()?;
                    Some(blam_tags::particle_model_strip(&tag).ok()?)
                }
                None => None,
            };
            Some(read_material(reader, root, strip.as_ref()).ok()?)
        }
        None => None,
    };
    let shader = root.descend("actual shader?");
    let flagged = |wanted: &str| has_flag(root, "appearance flags", wanted);
    let animated = |wanted: &str| has_flag(root, "animation flags", wanted);
    let first = root.read_int_any("first sequence index").unwrap_or(0).max(0) as usize;
    let offset = root.read_point2d("center offset");
    let start = tag_function_at(&root.descend("frame index")?, "Mapping").map_or(0.0, |function| function.evaluate(0.0, 0.0));
    let one_shot = animated("frame animation one shot");
    let (frames, corner) = match shader.filter(|_| rules.read_material.is_none()) {
        Some(shader) => read_frames(reader, shader, first, [offset.x, offset.y], start, one_shot),
        None => (vec![[0.0, 0.0, 1.0, 1.0]], [0.0, 0.0, 1.0, 1.0]),
    };
    let billboard = Billboard::named(&root.read_enum_option("particle billboard style").unwrap_or_default());
    let cheap = rules.cheap_shader && has_flag(root, "main flags", "uses cheap shader");
    const PROPERTIES: [&str; 7] =
        ["aspect ratio", "color", "intensity", "alpha", "frame index", "animation rate", "palette animation"];
    let mut properties = unallocated_properties(root, PROPERTIES)?;
    // Reach's `c_particle_definition::postprocess` (tool 0x140DD6FA0) takes
    // the mask of all seven before allocating them in this order. Its
    // attachments' scales come between, which the preview does not read.
    let mut seeds = if rules.reach_postprocess_seeds {
        SeedAllocator::reach(used_states_of(&properties) & REACH_ALLOCATED_STATES)
    } else {
        SeedAllocator::halo3()
    };
    properties.iter_mut().for_each(|property| property.allocate(&mut seeds));
    let [aspect, color, intensity, alpha, frame, rate, palette] = properties;
    let mut sprite_shader = match shader.filter(|_| rules.read_material.is_none()) {
        Some(shader) => read_sprite_shader(reader, shader)?,
        None => SpriteShader::plain(Gen3Blend::Additive),
    };
    // `default_vs`: Reach's constant self-illumination is at `V_ILLUM_EXPOSURE`.
    sprite_shader.add_at_illum = rules.adds_at_illum;
    if cheap {
        // `particle_render_fast.hlsl` warps nothing and lights no pixel.
        sprite_shader.sphere_warp = None;
        if sprite_shader.lighting == ParticleLighting::ReachSmooth {
            sprite_shader.lighting = ParticleLighting::None;
        }
    }
    Some(ParticleLook {
        billboard,
        still_moves: still_particles_move(reader.source.game(), billboard),
        flip_u: flagged("random u mirror"),
        flip_v: flagged("random v mirror"),
        random_rotation: flagged("random starting rotation"),
        intensity_fades: flagged("intensity affects alpha"),
        model,
        vertical: flagged("bitmap authored vertically"),
        dies_on_ground: has_flag(root, "main flags", "dies on structure collision"),
        dies_at_rest: has_flag(root, "main flags", "dies at rest"),
        tint_from_lightmap: flagged("tint from lightmap"),
        double_sided: flagged("double-sided"),
        attachments: elements(root, "attachments")
            .iter()
            .filter_map(|attachment| {
                let trigger = match attachment.read_enum_option("trigger").as_deref() {
                    Some("birth") => ParticleTrigger::Birth,
                    Some("death") => ParticleTrigger::Death,
                    Some("collision") => ParticleTrigger::Collision,
                    _ => return None,
                };
                let (group, path) = attachment.read_tag_ref_with_group("type")?;
                (group == u32::from_be_bytes(*b"effe") && !path.is_empty()).then_some((trigger, path))
            })
            .collect(),
        any_attachments: !elements(root, "attachments").is_empty(),
        carried_systems: None,
        death_scaled_by_speed: false,
        edge_fade: flagged("fade when viewed edge-on").then(|| {
            let real = |name: &str| root.read_real(name).unwrap_or(0.0);
            let range = real("angle fade range");
            let range = if range < 0.0001 { 30.0 } else { range };
            let radians = std::f32::consts::PI / 180.0;
            [(range * radians).recip(), real("angle fade cutoff") * radians]
        }),
        blur_aspect: flagged("motion blur").then(|| root.read_real("motion blur aspect scale").unwrap_or(0.0)),
        curvature: root.read_real("curvature").unwrap_or(0.0).clamp(0.0, 1.0) * 0.9999,
        one_shot,
        backwards: animated("can animate backwards"),
        corner,
        frames,
        sequences: Vec::new(),
        aspect,
        color,
        intensity,
        alpha,
        frame,
        rate,
        palette,
        shader: Arc::new(sprite_shader),
        h4,
        exposed: true,
        reach: rules.reach_look,
        cheap,
        used_states: seeds.used(),
    })
}

/// `holder`'s properties of these names, none yet seeded; none if any is missing.
pub(super) fn unallocated_properties<const N: usize>(holder: TagStruct<'_>, names: [&str; N]) -> Option<[ParticleProperty; N]> {
    let read = names.map(|name| ParticleProperty::unallocated(holder.descend(name)?));
    read.into_iter().collect::<Option<Vec<_>>>()?.try_into().ok()
}

/// What a definition's properties read, by Reach's state bits.
pub(super) fn used_states_of(properties: &[ParticleProperty]) -> u32 {
    properties.iter().fold(0, |used, property| used | property.used_states())
}
