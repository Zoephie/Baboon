//! Halo 2's particle systems: its `.particle` looks and emitters, read into
//! the shared particle field's terms, and the LOD its tool bakes.

use blam_tags::{TagFile, TagStruct};

use super::super::TagReader;
use super::super::common::fields::{elements, has_flag};
use super::super::gen3::materials::read_h3_bitmap;
use super::super::gen3::shaders::Gen3Blend;
use super::super::particles::{
    AlbedoConstants, Billboard, EmissionShape, Emitter, MOST_FRAMES, ParticleAlbedo, ParticleAxis,
    ParticleController, ParticleLook, ParticleProperty, ParticleTrigger, ParticleUpdate,
    SeedAllocator, SpriteShader, read_collider_of, read_movement, sprite_corner,
};
use crate::core::h2_pixel_shader::SamplerKind;
use std::sync::Arc;

/// Halo 2 tool's particle system postprocess (`sub_80D2A0`): out and its
/// feather default to 30 and 10, and only a system shown in any camera mode
/// gets the near pair, from how fast it emits. A feather left at nothing stays
/// so: `calc_lod_amount` (tag_test `sub_5D7990`) tests `near + feather` with
/// it, so there is no fade, and only multiplies by the inverse it bakes.
pub(in crate::app::model_preview) fn h2_baked_lod(
    raw: [f32; 4],
    any_camera: bool,
    rates: impl Iterator<Item = f32>,
) -> [f32; 4] {
    let empty = |value: f32| value.abs() < 0.0001;
    let [mut near, mut far, mut feather_in, mut feather_out] = raw;
    if empty(far) {
        far = 30.0;
    }
    if empty(feather_out) {
        feather_out = 10.0;
    }
    if any_camera {
        let busy = ((rates.map(|rate| rate / 30.0).sum::<f32>() - 2.0) / 3.0).clamp(0.0, 1.0);
        if empty(near) {
            near = busy * -0.1;
        }
        if empty(feather_in) {
            feather_in = (1.0 - busy) + busy * 3.0;
        }
    }
    [near, far, feather_in, feather_out]
}

/// A Halo 2 `particle_physics`' movements, its own or its template's.
fn h2_controllers(reader: &TagReader, movement: TagStruct<'_>) -> Vec<ParticleController> {
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| reader.load(&path, "particle_physics", b"pmov").ok());
    let own = elements(movement, "movements");
    let movements = if own.is_empty() {
        template
            .as_ref()
            .map_or(Vec::new(), |tag| elements(tag.root(), "movements"))
    } else {
        own
    };
    movements
        .into_iter()
        .filter_map(|entry| match entry.read_enum_option("type").as_deref() {
            Some("physics") => Some(ParticleController::Physics),
            Some("collider") => Some(ParticleController::Collider),
            _ => None,
        })
        .collect()
}

/// Halo 2's `sequences`: the sprites of `first`, on a plate of its own shape.
fn read_h2_frames(tag: &TagFile, first: usize) -> (Vec<[f32; 4]>, [f32; 4]) {
    let plate_aspect = elements(tag.root(), "bitmaps")
        .first()
        .map_or(1.0, |image| {
            let size = |name: &str| image.read_int_any(name).unwrap_or(1) as f32;
            size("width") / size("height")
        });
    let sequences = elements(tag.root(), "sequences");
    let sprites = sequences
        .get(first % sequences.len().max(1))
        .map_or(Vec::new(), |sequence| elements(*sequence, "sprites"));
    let rect_of = |sprite: &TagStruct<'_>| {
        ["left", "right", "top", "bottom"].map(|name| sprite.read_real(name).unwrap_or(0.0))
    };
    let frames: Vec<[f32; 4]> = sprites
        .iter()
        .take(MOST_FRAMES)
        .map(|sprite| {
            let [left, right, top, bottom] = rect_of(sprite);
            [left, top, right - left, bottom - top]
        })
        .collect();
    let (rect, center) = sprites
        .first()
        .map_or(([0.0, 1.0, 0.0, 1.0], [0.5, 0.5]), |sprite| {
            let registration = sprite.read_point2d("registration point");
            (rect_of(sprite), [registration.x, registration.y])
        });
    let frames = if frames.is_empty() {
        vec![[0.0, 0.0, 1.0, 1.0]]
    } else {
        frames
    };
    (frames, sprite_corner(rect, center, [0.0; 2], plate_aspect))
}

/// `effect_shader_*`'s pixel stage and blend, by the template that names the
/// pass, by name alone: the distortion one lives under `particle\` too.
fn h2_particle_pass(template: &str) -> Option<(ParticleAlbedo, Gen3Blend, bool)> {
    use ParticleAlbedo::{Diffuse, H2Distortion, H2MultiplyAdd, H2Tint};
    Some(
        match template.rsplit(['\\', '/']).next().unwrap_or_default() {
            "particle_additive" => (Diffuse, Gen3Blend::AddSrcTimesSrcAlpha, false),
            "particle_additive_tint" => (H2Tint, Gen3Blend::AddSrcTimesSrcAlpha, false),
            "particle_alpha_blend" => (Diffuse, Gen3Blend::AlphaBlend, false),
            "particle_alpha_blend_tint" => (H2Tint, Gen3Blend::AlphaBlend, false),
            "particle_alpha_multiply_add" => (H2MultiplyAdd, Gen3Blend::PreMultiplied, false),
            // Its alpha test passes anything above nothing, which the albedo does.
            "particle_alpha_blend_distortion" => (H2Distortion, Gen3Blend::AlphaBlend, false),
            "particle_alpha_test_no_lighting_fixed_for_glass" => {
                (Diffuse, Gen3Blend::AlphaBlend, true)
            }
            _ => return None,
        },
    )
}

/// A Halo 2 `.particle`, and the two properties of a quad that Halo 3 moved to
/// its emitters.
/// A Halo 2 system's look and emitters, for the particle reader's table.
pub(in crate::app::model_preview) fn read_h2_system_parts(
    reader: &TagReader,
    system: TagStruct<'_>,
    path: &str,
) -> Option<(Arc<ParticleLook>, Vec<Arc<Emitter>>)> {
    let flagged = |wanted: &str| has_flag(system, "flags", wanted);
    let (look, scale, rotation) = read_h2_look(reader, path)?;
    let read = |emitter: &TagStruct<'_>| {
        read_h2_emitter(reader, *emitter, scale.clone(), rotation.clone(), &flagged)
    };
    Some((
        Arc::new(look),
        elements(system, "emitters")
            .iter()
            .filter_map(read)
            .map(Arc::new)
            .collect(),
    ))
}

/// [`h2_baked_lod`], as the particle reader's table holds it.
pub(in crate::app::model_preview) fn h2_baked_lod_of(
    raw: [f32; 4],
    any_camera: bool,
    rates: &mut dyn Iterator<Item = f32>,
) -> [f32; 4] {
    h2_baked_lod(raw, any_camera, rates)
}

pub(in crate::app::model_preview) fn read_h2_look(
    reader: &TagReader,
    path: &str,
) -> Option<(ParticleLook, ParticleProperty, ParticleProperty)> {
    let tag = reader.load(path, "particle", b"prt3").ok()?;
    let root = tag.root();
    let template = root.read_tag_ref_path("shader template")?;
    let (albedo, blend, alpha_test) =
        h2_particle_pass(template.trim_end_matches(".shader_template"))?;
    let parameters = elements(root, "shader parameters");
    let parameter = |name: &str| {
        parameters
            .iter()
            .find(|entry| entry.read_string_id("name").as_deref() == Some(name))
    };
    let distorts = albedo == ParticleAlbedo::H2Distortion;
    let map_name = if distorts {
        "bump_map"
    } else {
        "alpha_map_texture"
    };
    let map = parameter(map_name)?
        .read_tag_ref_path("bitmap")
        .filter(|path| !path.is_empty())?;
    let plate = reader.load(&map, "bitmap", b"bitm").ok()?;
    let flagged = |wanted: &str| has_flag(root, "flags", wanted);
    let first = root
        .read_int_any("first sequence index")
        .unwrap_or(0)
        .max(0) as usize;
    let (frames, corner) = read_h2_frames(&plate, first);
    // `sub_53CEA0`: `first + random % count`, held inside the bitmap's.
    let sequences = if flagged("select random sequence") {
        let count = root.read_int_any("sequence count").unwrap_or(1).max(1) as usize;
        let last = elements(plate.root(), "sequences").len().saturating_sub(1);
        (0..count)
            .map(|at| read_h2_frames(&plate, (first + at).min(last)))
            .collect()
    } else {
        Vec::new()
    };
    // The distortion's pixel shader reads `distortion_scale` where the others
    // read `modulation_factor`; its template's default is 1.
    let modulation = if distorts {
        parameter("distortion_scale")
            .and_then(|entry| entry.read_real("const value"))
            .unwrap_or(1.0)
    } else {
        parameter("modulation_factor")
            .and_then(|entry| entry.read_real("const value"))
            .unwrap_or(0.0)
    };
    let shader = SpriteShader {
        albedo,
        alpha_test,
        frame_blend: !flagged("disable frame blending"),
        base_map: Some(read_h3_bitmap(reader, &map, 0, SamplerKind::Texture2D)?),
        constants: AlbedoConstants {
            modulation,
            ..Default::default()
        },
        ..SpriteShader::plain(blend)
    };
    let mut seeds = SeedAllocator::default();
    let billboard = Billboard::named(
        &root
            .read_enum_option("particle billboard style")
            .unwrap_or_default(),
    );
    let mut property = |name: &str| ParticleProperty::named(root, name, &mut seeds);
    let (color, alpha, scale, rotation, frame) = (
        property("color")?,
        property("alpha")?,
        property("scale")?,
        property("rotation")?,
        property("frame index")?,
    );
    let look = ParticleLook {
        billboard,
        still_moves: false,
        flip_u: flagged("random u mirror"),
        flip_v: flagged("random v mirror"),
        random_rotation: false,
        intensity_fades: false,
        model: None,
        vertical: flagged("bitmap authored vertically"),
        dies_on_ground: flagged("dies on structure collision"),
        dies_at_rest: flagged("dies at rest"),
        // The doubled tint of what is lit comes from a level.
        tint_from_lightmap: false,
        double_sided: true,
        // `sub_61AE40`: a death's `effect` is made where the particle died, and
        // a collision's where it first struck (`sub_53DF20`). A material effect
        // needs the surface's material, which the preview's ground has none
        // of, and a sound draws nothing.
        attachments: [
            ("death effect", ParticleTrigger::Death),
            ("collision effect", ParticleTrigger::Collision),
        ]
        .into_iter()
        .filter_map(|(field, trigger)| {
            let (group, path) = root.read_tag_ref_with_group(field)?;
            (group == u32::from_be_bytes(*b"effe") && !path.is_empty()).then_some((trigger, path))
        })
        .collect(),
        // Read only for Halo 3 and ODST: Halo 2's emitters are all on the processor.
        any_attachments: false,
        carried_systems: (!elements(root, "attached particle systems").is_empty())
            .then(|| path.to_owned()),
        death_scaled_by_speed: true,
        edge_fade: None,
        blur_aspect: None,
        curvature: 0.0,
        one_shot: flagged("frame animation one shot"),
        backwards: flagged("can animate backwards"),
        corner,
        frames,
        sequences,
        aspect: ParticleProperty::fixed(1.0)?,
        color,
        intensity: ParticleProperty::fixed(1.0)?,
        alpha,
        frame,
        rate: ParticleProperty::fixed(0.0)?,
        palette: ParticleProperty::fixed(0.0)?,
        shader: Arc::new(shader),
        h4: None,
        exposed: false,
        reach: false,
        cheap: false,
        used_states: 0,
    };
    Some((look, scale, rotation))
}

/// A Halo 2 emitter: Halo 3's without what it added, its rates in ticks and
/// degrees, and `size` and `rotation` scaled by the `.particle`'s.
pub(in crate::app::model_preview) fn read_h2_emitter(
    reader: &TagReader,
    emitter: TagStruct<'_>,
    scale: ParticleProperty,
    rotation: ParticleProperty,
    flagged: &impl Fn(&str) -> bool,
) -> Option<Emitter> {
    let mut seeds = SeedAllocator::default();
    let shape = EmissionShape::named(
        &emitter
            .read_enum_option("emission shape")
            .unwrap_or_default(),
    )?;
    let mut property = |name: &str| ParticleProperty::named(emitter, name, &mut seeds);
    let (radius, angle, rate, lifespan) = (
        property("emission radius")?,
        property("emission angle")?,
        property("particle emission rate")?.scaled(30.0),
        property("particle lifespan")?,
    );
    let (speed, rotation_rate, size, tint, alpha) = (
        property("particle velocity")?,
        property("particle angular velocity")?.scaled(1.0 / 360.0),
        property("particle size")?,
        property("particle tint")?,
        property("particle alpha")?,
    );
    let offset = emitter.read_point3d("translational offset");
    let turn = emitter.read_euler2d("relative direction");
    let (sin_yaw, cos_yaw) = turn.yaw.sin_cos();
    let (sin_pitch, cos_pitch) = turn.pitch.sin_cos();
    let movement = emitter
        .read_tag_ref_path("particle physics")
        .filter(|path| !path.is_empty())
        .and_then(|path| reader.load(&path, "particle_physics", b"pmov").ok());
    let physics = movement
        .as_ref()
        .and_then(|tag| read_movement(reader, tag.root(), &mut seeds));
    let collider = movement
        .as_ref()
        .and_then(|tag| read_collider_of(reader, tag.root(), &mut seeds));
    // In the order the tag lists them, as the game runs its movements.
    let controllers = movement
        .as_ref()
        .map_or_else(Vec::new, |tag| h2_controllers(reader, tag.root()));
    let controllers = controllers
        .into_iter()
        .filter(|controller| match controller {
            ParticleController::Physics => physics.is_some(),
            ParticleController::Collider => collider.is_some(),
            ParticleController::Flock => false,
        })
        .collect();
    Some(Emitter {
        h4: None,
        update: ParticleUpdate::Cpu,
        card_lerps: false,
        card_at_rest: false,
        spawn_slack: 0.0001,
        controllers,
        shape,
        points: Vec::new(),
        scroll: [0.0; 2],
        axis: ParticleAxis::named(""),
        reference_axis: 0,
        axis_angle: ParticleProperty::fixed(0.0)?,
        offset: ParticleProperty::fixed_vector([offset.x, offset.y, offset.z])?,
        direction: ParticleProperty::fixed_vector([
            cos_yaw * cos_pitch,
            sin_yaw * cos_pitch,
            sin_pitch,
        ])?,
        radius,
        angle,
        starting: ParticleProperty::fixed(0.0)?,
        most: ParticleProperty::fixed(0.0)?,
        rate,
        lifespan,
        acceleration: ParticleProperty::fixed_vector([0.0; 3])?,
        speed,
        rotation,
        rotation_rate,
        size,
        scale,
        tint,
        alpha,
        black_point: ParticleProperty::fixed(0.0)?,
        physics,
        flock: None,
        collider,
        one_at_a_time: flagged("looping particle"),
        persistent: flagged("persistent particle"),
        bunched: !flagged("spread between ticks"),
        rotation_given: true,
        volume_random: false,
        axis_scale: [1.0; 3],
        turbulence: None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::particles::lod_amount;
    use super::*;

    #[test]
    fn halo_2_passes_come_from_their_templates_and_distortion_has_none() {
        let pass = |name: &str| {
            h2_particle_pass(&format!("shaders\\shader_templates\\transparent\\{name}"))
        };
        assert_eq!(
            pass("particle_additive"),
            Some((
                ParticleAlbedo::Diffuse,
                Gen3Blend::AddSrcTimesSrcAlpha,
                false
            ))
        );
        assert_eq!(
            pass("particle_alpha_blend_tint"),
            Some((ParticleAlbedo::H2Tint, Gen3Blend::AlphaBlend, false))
        );
        assert_eq!(
            pass("particle_alpha_multiply_add"),
            Some((
                ParticleAlbedo::H2MultiplyAdd,
                Gen3Blend::PreMultiplied,
                false
            ))
        );
        assert_eq!(
            pass("particle_alpha_blend_distortion"),
            Some((ParticleAlbedo::H2Distortion, Gen3Blend::AlphaBlend, false))
        );
    }

    #[test]
    fn a_halo_2_lod_fades_in_near_the_camera_only_when_shown_in_any_camera_mode() {
        assert_eq!(
            h2_baked_lod([0.0; 4], true, [].into_iter()),
            [0.0, 30.0, 1.0, 10.0]
        );
        assert_eq!(
            h2_baked_lod([0.0; 4], true, [150.0].into_iter()),
            [-0.1, 30.0, 3.0, 10.0]
        );
        assert_eq!(
            h2_baked_lod([0.0; 4], false, [150.0].into_iter()),
            [0.0, 30.0, 0.0, 10.0],
            "not for one camera"
        );
        assert_eq!(
            h2_baked_lod([1.0, 5.0, 2.0, 3.0], true, [150.0].into_iter()),
            [1.0, 5.0, 2.0, 3.0]
        );
        // The SMG's muzzle burst, a hand's length from the eye.
        let one_camera = Some(h2_baked_lod([0.0; 4], false, [30.0].into_iter()));
        assert_eq!(lod_amount(one_camera, 0.8), 1.0);
        let any_camera = Some(h2_baked_lod([0.0; 4], true, [30.0].into_iter()));
        assert!((lod_amount(any_camera, 0.8) - 0.8).abs() < 1e-6);
    }
}
