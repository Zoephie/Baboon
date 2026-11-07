//! The shield impact Reach and Halo 4 share: a `shield_impact` tag's
//! settings, the constants `c_render_shield::render_object_internal` submits
//! from them each frame, which damage section's impact an object takes, and
//! which parts draw it. See `docs/reach-preview-research/shield.md`.

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::super::common::fields::{elements, has_flag};
use super::super::functions::FunctionValues;
use super::super::gen3::shaders::{H3Texels, H3Texture};
use super::super::materials::decode_h2_image;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::export::geometry::load_referenced_tag_from_source;
use crate::core::source::TagSource;
use crate::app::model_preview::common::named_function::NamedFunction;

/// `shield_impact` `flags`.
const PROCEDURAL_UV: &str = "use procedural uv generation";
const RENDER_ALWAYS: &str = "render always";
/// `s_shield_parameters::postprocess`'s floor under a range it divides by.
const SMALLEST: f32 = 1e-4;

/// The vertex stage's two noise maps, which the source names as the pixel
/// stage's but the game binds the oscillation textures to.
pub(crate) const OSCILLATION: [&str; 2] = ["ReachShieldOscillation_texture1", "ReachShieldOscillation_texture2"];
pub(crate) const PLASMA: [&str; 2] =
    ["GlobalTexture_shield_impact_noise_texture1", "GlobalTexture_shield_impact_noise_texture2"];
/// What the preview's depth reads back as world distance with.
pub(crate) const DEPTH_DECODE: &str = "reach_shield_depth_decode";
pub(crate) const WORLD_SCALE: &str = "reach_shield_world_scale";

/// A `shield_impact` function: `shield_color_function_struct` and
/// `shield_scalar_function_struct`.
#[derive(Debug, Clone)]
struct ShieldChannel(NamedFunction);

impl std::ops::Deref for ShieldChannel {
    type Target = NamedFunction;

    fn deref(&self) -> &NamedFunction {
        &self.0
    }
}

impl ShieldChannel {
    fn read(root: TagStruct<'_>, name: &str) -> Option<Self> {
        NamedFunction::read(root, name).map(Self)
    }

    /// `eval_string_id`: what `render_object_internal` knows by name, then
    /// the object's own functions.
    fn lookup(name: &str, state: &ShieldState<'_>) -> Option<f32> {
        Some(match name {
            "" => return None,
            "recent_shield_damage" => state.recent,
            "current_shield_damage" => state.current,
            "shield_intensity" => state.intensity,
            "render_time" => state.time,
            "event_time" => 0.0,
            other => state.values.and_then(|values| values.get(other)).copied().unwrap_or(0.0),
        })
    }

    /// `eval_scalar_function`'s input and range: time, and a fresh random
    /// number on every call, when none is named.
    fn sample(&self, state: &ShieldState<'_>, salt: u32) -> (f32, f32) {
        let input = Self::lookup(&self.input, state).unwrap_or(state.time);
        let range = Self::lookup(&self.range, state).unwrap_or_else(|| real_random(state.time, salt));
        (input, range)
    }

    fn scalar(&self, state: &ShieldState<'_>, salt: u32) -> f32 {
        let (input, range) = self.sample(state, salt);
        self.function.evaluate(input, range)
    }

    fn color(&self, state: &ShieldState<'_>, salt: u32) -> [f32; 3] {
        let (input, range) = self.sample(state, salt);
        let color = self.function.evaluate_color(input, range);
        [color.red, color.green, color.blue]
    }
}

/// A stand-in for `real_random`, new each frame and each call site.
pub(crate) fn real_random(time: f32, salt: u32) -> f32 {
    let mut bits = time.to_bits() ^ salt.wrapping_mul(0x9E37_79B9);
    bits ^= bits >> 16;
    bits = bits.wrapping_mul(0x7FEB_352D);
    bits ^= bits >> 15;
    bits = bits.wrapping_mul(0x846C_A68B);
    bits ^= bits >> 16;
    (bits >> 8) as f32 / (1u32 << 24) as f32
}

/// What `render_object_internal` reads its functions against.
pub(crate) struct ShieldState<'a> {
    /// `state[1]`, the recent impact.
    recent: f32,
    /// `state[2]`, the current impact.
    current: f32,
    /// `state[3]`.
    intensity: f32,
    time: f32,
    values: Option<&'a FunctionValues>,
}

/// A `shield_impact` as `s_shield_parameters::postprocess` leaves it.
#[derive(Debug)]
pub(crate) struct ReachShieldSettings {
    pub(in crate::app::model_preview) procedural: bool,
    render_always: bool,
    recent_intensity: f32,
    current_intensity: f32,
    scroll_speed: f32,
    oscillation_tiling: f32,
    oscillation_scroll: f32,
    edge_scales: [f32; 4],
    edge_offsets: [f32; 4],
    plasma_scales: [f32; 4],
    depth_fade_params: [f32; 4],
    /// The procedural pass's single fade, in `depth_constants.w`.
    inverse_depth_fade: f32,
    edge_glow: Option<(ShieldChannel, ShieldChannel)>,
    plasma_center: Option<(ShieldChannel, ShieldChannel)>,
    plasma_edge: Option<(ShieldChannel, ShieldChannel)>,
    extrusion: Option<ShieldChannel>,
    oscillation_amplitude: Option<ShieldChannel>,
    pub(crate) textures: Vec<H3Texture>,
}

/// The edge radii as `postprocess` turns them into scales and offsets: each
/// `1 - radius`, and the line through outer→0, center→1 and center→1, inner→0.
fn edge_lines(outer: f32, center: f32, inner: f32) -> ([f32; 2], [f32; 2]) {
    let (o, c, i) = (1.0 - outer, 1.0 - center, 1.0 - inner);
    ([1.0 / (c - o), 1.0 / (c - i)], [-o / (c - o), -i / (c - i)])
}

impl ReachShieldSettings {
    pub(crate) fn read(source: &TagSource, path: &str) -> Option<Self> {
        let tag = load_referenced_tag_from_source(source, path, "shield_impact", b"shit").ok()?;
        let root = tag.root();
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let flagged = |wanted: &str| has_flag(root, "flags", wanted);
        let ([outer, inner], [outer_offset, inner_offset]) =
            edge_lines(real("Outer Fade Radius"), real("Center Radius"), real("Inner Fade Radius"));
        let ([plasma_outer, plasma_inner], [plasma_outer_offset, plasma_inner_offset]) = edge_lines(
            real("Plasma Outer Fade Radius"),
            real("Plasma Center Radius"),
            real("Plasma Inner Fade Radius"),
        );
        let tiling = real("Tiling Scale");
        let (edge_sharpness, center_sharpness) = (real("Edge Sharpness"), real("Center Sharpness"));
        let inverse_depth_fade = 1.0 / real("Depth Fade Range").max(SMALLEST);
        let inverse_plasma_fade = 1.0 / real("Plasma Depth Fade Range").max(SMALLEST);
        let pair = |color: &str, intensity: &str| {
            Some((ShieldChannel::read(root, color)?, ShieldChannel::read(root, intensity)?))
        };
        let mut textures = Vec::new();
        for (field, uniform) in [
            ("Plasma Noise Texture 0", PLASMA[0]),
            ("Plasma Noise Texture 1", PLASMA[1]),
            ("Oscillation Texture 0", OSCILLATION[0]),
            ("Oscillation Texture 1", OSCILLATION[1]),
        ] {
            let Some(bitmap) = root.read_tag_ref_path(field).filter(|bitmap| !bitmap.is_empty()) else {
                continue;
            };
            let Some(image) = decode_h2_image(source, &bitmap, SamplerKind::Texture2D, 0) else {
                continue;
            };
            textures.push(H3Texture {
                uniform: uniform.to_owned(),
                image: Arc::new(image),
                texels: H3Texels::Linear,
                mips: Arc::default(),
                sampler: None,
                frames: Vec::new(),
            });
        }
        Some(Self {
            procedural: flagged(PROCEDURAL_UV),
            render_always: flagged(RENDER_ALWAYS),
            recent_intensity: real("Recent Damage Intensity"),
            current_intensity: real("Current Damage Intensity"),
            scroll_speed: real("Scroll Speed"),
            oscillation_tiling: real("Oscillation Tiling Scale"),
            oscillation_scroll: real("Oscillation Scroll Speed"),
            edge_scales: [outer, inner, plasma_outer, plasma_inner],
            edge_offsets: [outer_offset, inner_offset, plasma_outer_offset, plasma_inner_offset],
            plasma_scales: [tiling, tiling, edge_sharpness - center_sharpness, center_sharpness],
            depth_fade_params: [inverse_depth_fade, inverse_plasma_fade, 0.0, 0.0],
            inverse_depth_fade,
            edge_glow: pair("Edge Glow Color", "Edge Glow Intensity"),
            plasma_center: pair("Plasma Center Color", "Plasma Center Intensity"),
            plasma_edge: pair("Plasma Edge Color", "Plasma Edge Intensity"),
            extrusion: ShieldChannel::read(root, "Extrusion Amount"),
            oscillation_amplitude: ShieldChannel::read(root, "Oscillation Amplitude"),
            textures,
        })
    }

    /// `object_get_active_shield_render_parameters`: shown while the shield
    /// has taken damage, or always if it says so.
    pub(crate) fn showing(&self, damage: f32) -> bool {
        damage > 0.0 || self.render_always
    }

    /// The state `render_object_internal` decompresses, at the moment of an
    /// impact of `damage`: recent and current alike.
    pub(crate) fn state<'a>(&self, damage: f32, time: f32, values: Option<&'a FunctionValues>) -> ShieldState<'a> {
        let intensity = self.recent_intensity * damage + self.current_intensity * damage;
        ShieldState { recent: damage, current: damage, intensity, time, values }
    }

    /// Every constant the pass reads, but the depth decode, by uniform.
    /// `center` is the bounding sphere's, where a shield with no hit puts it.
    pub(crate) fn values(&self, state: &ShieldState<'_>, center: [f32; 3]) -> HashMap<&'static str, [f32; 4]> {
        let tinted = |pair: &Option<(ShieldChannel, ShieldChannel)>, salt: u32| {
            pair.as_ref().map_or([0.0; 3], |(color, intensity)| {
                let scale = intensity.scalar(state, salt + 1);
                color.color(state, salt).map(|channel| channel * scale)
            })
        };
        let scalar = |channel: &Option<ShieldChannel>, salt: u32| channel.as_ref().map_or(0.0, |channel| channel.scalar(state, salt));
        let t = state.time;
        let scroll = t * self.scroll_speed;
        let edge_glow = tinted(&self.edge_glow, 0);
        let plasma = tinted(&self.plasma_center, 2);
        let plasma_edge = tinted(&self.plasma_edge, 4);
        let k = t * self.oscillation_scroll;
        let mut values = HashMap::new();
        values.insert("plasma_offsets", [scroll / 12.0, scroll / 13.0, -scroll / 11.0, scroll / 17.0]);
        values.insert("edge_glow", [edge_glow[0], edge_glow[1], edge_glow[2], 1.0]);
        values.insert("plasma_color", [plasma[0], plasma[1], plasma[2], 1.0]);
        values.insert(
            "plasma_edge_color",
            [plasma_edge[0] - plasma[0], plasma_edge[1] - plasma[1], plasma_edge[2] - plasma[2], 1.0],
        );
        values.insert("edge_scales", self.edge_scales);
        values.insert("edge_offsets", self.edge_offsets);
        values.insert("plasma_scales", self.plasma_scales);
        values.insert("depth_fade_params", self.depth_fade_params);
        values.insert("depth_constants", [0.0, 0.0, 0.0, self.inverse_depth_fade]);
        values.insert(
            "vertex_params",
            [scalar(&self.extrusion, 6), 0.0, scalar(&self.oscillation_amplitude, 7), self.oscillation_tiling],
        );
        values.insert("vertex_params2", [k, k, -1.07 * k, 1.08 * k]);
        // No hit: the impact sits at the centre, too small to reach anything.
        values.insert("impact0_params", [center[0], center[1], center[2], SMALLEST]);
        values.insert("impact0_color", [0.0; 4]);
        values
    }
}

/// `object_get_active_shield_render_parameters`' choice of `shield_impact`,
/// third person: from the last damage section back, the first that is a
/// shield with something to render and is live at rest (not `starts
/// inactive`). A section with an empty `rendering` takes the model's own
/// override, as `model_postprocess_build_damage_new` fills it.
pub(crate) fn shield_impact_path(model: &TagFile) -> Option<String> {
    let root = model.root();
    let info = root.descend("damage info")?;
    if info.read_real("maximum vitality").unwrap_or(0.0) <= 0.0 {
        return None;
    }
    let reference = |holder: TagStruct<'_>, name: &str| holder.read_tag_ref_path(name).filter(|path| !path.is_empty());
    let sections = elements(info, "damage sections");
    let chosen = sections.into_iter().rev().find(|section| {
        let starts_inactive = has_flag(*section, "flags", "starts inactive");
        let shield = section.read_string_id("shield material name").is_some_and(|name| !name.is_empty());
        let vitality = section.read_real("vitality percentage").unwrap_or(0.0) > 0.0;
        // An empty `rendering` gains an element in postprocess, and so counts.
        let renders = !elements(*section, "rendering").is_empty() || reference(root, "shield impact parameter override").is_some();
        shield && vitality && renders && !starts_inactive
    })?;
    match elements(chosen, "rendering").first() {
        Some(rendering) => reference(*rendering, "3rd person impact parameters"),
        None => reference(root, "shield impact parameter override"),
    }
}

/// Whether a part of `material` takes the shield: `render_model_postprocess`
/// leaves flag 0x40 off a part whose material only renders for shields, and
/// off an ordinary part whose material does not refuse them. Decals take it.
pub(crate) fn draws_shield(shield_only: bool, never_shield: bool, decal: bool) -> bool {
    shield_only || (!never_shield && !decal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::export::geometry::load_referenced_tag_from_source;

    #[test]
    fn edge_radii_become_the_lines_through_their_fades() {
        // spartan_3p's: the outer fade at 0.65 → 0, the center at 0.25 → 1.
        let ([outer, inner], [outer_offset, inner_offset]) = edge_lines(0.65, 0.25, 0.0);
        assert!((outer - 2.5).abs() < 1e-5 && (outer_offset + 0.875).abs() < 1e-5);
        assert!((inner + 4.0).abs() < 1e-5 && (inner_offset - 4.0).abs() < 1e-5);
        let at = |c: f32| (c * outer + outer_offset).min(c * inner + inner_offset);
        assert!((at(0.75) - 1.0).abs() < 1e-5, "full at the center");
        assert!(at(0.35).abs() < 1e-5 && at(1.0).abs() < 1e-5, "nothing past either fade");
    }

    #[test]
    fn a_part_takes_the_shield_unless_its_material_refuses() {
        assert!(draws_shield(false, false, false));
        assert!(!draws_shield(false, true, false));
        assert!(draws_shield(true, false, false));
        assert!(!draws_shield(false, false, true));
    }

    #[test]
    fn real_random_moves_with_time_and_call() {
        let a = real_random(1.0, 0);
        assert!((0.0..1.0).contains(&a));
        assert_ne!(a, real_random(1.0, 1));
        assert_ne!(a, real_random(2.0, 0));
    }

    fn kit() -> Option<TagSource> {
        let root = std::env::var("BLAM_TEST_HREK").ok()?;
        Some(TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloReach),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        })
    }

    fn model(source: &TagSource, path: &str) -> TagFile {
        load_referenced_tag_from_source(source, path, "model", b"hlmt").unwrap()
    }

    /// With `BLAM_TEST_HREK`: the Spartan and the Elite take their plain
    /// shields, past the armor lock and engineer sections that start inactive;
    /// a model with no shield section takes none.
    #[test]
    fn the_kits_bipeds_take_their_live_shield() {
        let Some(source) = kit() else {
            return;
        };
        let spartan = model(&source, "objects\\characters\\spartans\\spartans");
        assert_eq!(shield_impact_path(&spartan).as_deref(), Some("fx\\shield_impacts\\spartan_3p"));
        let elite = model(&source, "objects\\characters\\elite\\elite");
        assert_eq!(shield_impact_path(&elite).as_deref(), Some("fx\\shield_impacts\\elite_3p"));
        let grunt = model(&source, "objects\\characters\\grunt\\grunt");
        assert_eq!(shield_impact_path(&grunt), None);
    }

    /// With `BLAM_TEST_HREK`: spartan_3p as the 360's postprocess leaves
    /// it, and hidden at rest.
    #[test]
    fn spartan_3p_postprocesses_as_the_game_does() {
        let Some(source) = kit() else {
            return;
        };
        let settings = ReachShieldSettings::read(&source, "fx\\shield_impacts\\spartan_3p").unwrap();
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3);
        assert!(close(settings.edge_scales, [2.5, -4.0, 4.0, -4.0]), "{:?}", settings.edge_scales);
        assert!(close(settings.edge_offsets, [-0.875, 4.0, 0.0, 2.0]), "{:?}", settings.edge_offsets);
        assert!(close(settings.plasma_scales, [1.75, 1.75, -55.0, 60.0]), "{:?}", settings.plasma_scales);
        assert!(close(settings.depth_fade_params, [2.0, 6.667, 0.0, 0.0]), "{:?}", settings.depth_fade_params);
        assert_eq!(settings.textures.len(), 4);
        assert!(!settings.showing(0.0));
        assert!(settings.showing(0.5));
        let state = settings.state(1.0, 2.0, None);
        assert!((state.intensity - 1.5).abs() < 1e-6, "0.5 recent and 1 current");
    }
}
