//! Halo 4's particles drawn through their material: what a particle's
//! `actual material` and bitmap give its draw, and each particle packed as
//! `pack_particle_state` packs it for the vertex shader to unpack. See
//! `docs/h4-preview-research/particles.md`.

use blam_tags::TagStruct;

use super::ce_attachments::elements;
use super::h2_passes::tag_function_at;
use super::materials::{H4EffectMaterial, resolve_h4_effect_material};
use crate::app::export::geometry::load_referenced_tag_from_source;
use crate::core::source::TagSource;

/// `alpha_blend_mode_enum` values the draw tells apart.
const BLEND_OPAQUE: u8 = 0;
const BLEND_ADDITIVE: u8 = 1;
const BLEND_ALPHA: u8 = 3;
const BLEND_ADD_SRC_TIMES_SRCALPHA: u8 = 9;

/// The runtime appearance bits `c_particle_definition::postprocess` (X360
/// 0x831F0A18) adds over the tag's (`particles.md` §2.4).
const LIGHTMAP_LIT: u32 = 0x8000;
const DEPTH_FADE: u32 = 0x10000;
const DISTORTION: u32 = 0x20000;
const PARTICLE_MODEL: u32 = 0x80000;
const OPAQUE: u32 = 0x100000;
const SPHERE_WARP: u32 = 0x200000;
const NEW_SCHOOL_FRAMES: u32 = 0x400000;
/// Main flag 0x40: the particle has no attachments.
const NO_ATTACHMENTS: u32 = 0x40;
/// Animation flag 4: the sprite sequence is not animated.
const STATIC_FRAME: u32 = 4;

/// A tag's `random u mirror` and `random v mirror`.
const RANDOM_MIRRORS: u32 = 0x3;

/// Bitmap flags: `generate tight bounds`, `generate black-point tight bounds`.
const TIGHT_BOUNDS: i128 = 0x200;
const BLACK_POINT_TIGHT_BOUNDS: i128 = 0x4000;

/// The default corners of every custom-vertex set.
const UNIT_QUAD: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// A particle tag's draw: its material and what the game derives from the
/// tag and its first bitmap at load.
#[derive(Debug)]
pub(crate) struct H4ParticleRender {
    pub(crate) material: H4EffectMaterial,
    /// `vsRenderState.flags_main_appearance_animation`.
    pub(crate) flags: [u32; 3],
    pub(crate) billboard: u32,
    /// The postprocessed curvature.
    pub(crate) curvature: f32,
    /// `1 / angle fade range`, and its cutoff, in radians.
    pub(crate) edge_fade: [f32; 2],
    /// `vs_sprite_offset_and_scale`.
    pub(crate) sprite: [f32; 4],
    /// `vs_sprite_list_count_and_max.x`, and `vs_sprite_list_sprites`.
    pub(crate) sprite_count: f32,
    pub(crate) sprites: Vec<[f32; 4]>,
    /// `customVerts`: five sets of four corners.
    pub(crate) custom_verts: [[[f32; 2]; 4]; 5],
    /// `psTintFactor.x`.
    pub(crate) tint_factor: f32,
}

impl H4ParticleRender {
    pub(crate) fn appearance(&self) -> u32 {
        self.flags[1]
    }

    /// `psDepthFadeEnabled`, `psSphereWarpEnabled`, `psNewSchoolFrameIndex`.
    pub(crate) fn pixel_bools(&self) -> [bool; 3] {
        let appearance = self.appearance();
        [
            appearance & DEPTH_FADE != 0,
            appearance & SPHERE_WARP != 0,
            appearance & NEW_SCHOOL_FRAMES != 0,
        ]
    }
}

/// `c_particle_definition::postprocess` and `postprocess_frame_animation`
/// (X360 0x831F1FE0) for the particle at `root`.
pub(crate) fn read_h4_particle_render(
    source: &TagSource,
    root: TagStruct<'_>,
    has_model: bool,
) -> Result<H4ParticleRender, String> {
    let material = root
        .descend("actual material?")
        .ok_or("no actual material")?;
    let material = blam_tags::material::Material::from_struct(&material);
    let pass = if has_model {
        "particle_model"
    } else {
        "particle"
    };
    let material = resolve_h4_effect_material(source, &material, pass)?;
    let int = |name: &str| root.read_int_any(name).unwrap_or(0) as u32;
    let mut main = int("main flags");
    if elements(root, "attachments").is_empty() {
        main |= NO_ATTACHMENTS;
    }
    let mut appearance = int("appearance flags") & 0x3FFF;
    if material.flag("LightingPerParticle") || material.flag("LightingSmooth") {
        appearance |= LIGHTMAP_LIT;
    }
    if material.real("DepthFadeRange").unwrap_or(0.5) >= 0.025 {
        appearance |= DEPTH_FADE;
    }
    if material.pass.pixel.contains("psDistortionScreenConstants") {
        appearance |= DISTORTION;
    }
    if has_model {
        appearance |= PARTICLE_MODEL;
    }
    if material.blend_mode == BLEND_OPAQUE {
        appearance |= OPAQUE;
    }
    if material.real("SphereWarpStrength").unwrap_or(0.0) > 0.01 {
        appearance |= SPHERE_WARP;
    }
    if material.flag("NewSchoolFrameIndex") {
        appearance |= NEW_SCHOOL_FRAMES;
    }
    let mut animation = int("animation flags");
    let one_shot = animation & 1 != 0;
    let function = |name: &str| {
        root.descend(name)
            .and_then(|holder| tag_function_at(&holder, "Mapping"))
    };
    let frame_index = function("frame index");
    let rate = function("animation rate");
    let start = frame_index
        .as_ref()
        .map_or(0.0, |function| function.evaluate(0.0, 0.0));
    let changes = frame_index
        .as_ref()
        .is_some_and(|function| function.as_constant().is_none())
        || rate
            .as_ref()
            .is_some_and(|function| function.as_constant().is_none_or(|value| value != 0.0));
    let first = root
        .read_int_any("first sequence index")
        .unwrap_or(0)
        .max(0) as usize;
    let offset = root.read_point2d("center offset");
    let bitmap = material
        .first_bitmap
        .as_deref()
        .and_then(|path| load_referenced_tag_from_source(source, path, "bitmap", b"bitm").ok());
    let placement = sprite_placement(
        bitmap.as_ref().map(|tag| tag.root()),
        first,
        start,
        one_shot,
        changes,
        [offset.x, offset.y],
    );
    if !placement.animated {
        animation |= STATIC_FRAME;
    }
    let tags_mirror = int("appearance flags") & RANDOM_MIRRORS != 0;
    let bounded = matches!(
        material.blend_mode,
        BLEND_ADDITIVE | BLEND_ALPHA | BLEND_ADD_SRC_TIMES_SRCALPHA
    ) && !tags_mirror;
    let custom_verts = if bounded {
        bitmap
            .as_ref()
            .map_or([UNIT_QUAD; 5], |tag| custom_verts(tag.root()))
    } else {
        [UNIT_QUAD; 5]
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let range = real("angle fade range");
    let range = if range < 1e-4 { 30.0 } else { range };
    Ok(H4ParticleRender {
        flags: [main, appearance, animation],
        billboard: root
            .read_int_any("particle billboard style")
            .unwrap_or(0)
            .max(0) as u32,
        curvature: (1.0 - 1e-4) * real("curvature").clamp(0.0, 1.0),
        edge_fade: [
            1.0 / range.to_radians(),
            real("angle fade cutoff").to_radians(),
        ],
        sprite: placement.offset_and_scale,
        sprite_count: placement.sprites.len().max(1) as f32,
        sprites: placement.sprites,
        custom_verts,
        // Tags whose layout predates it read 0: modulate.
        tint_factor: root.read_int_any("tint factor").unwrap_or(0) as f32,
        material,
    })
}

/// What `postprocess_frame_animation` builds: the quad's placement, the
/// frames the vertex shader picks among, and whether they animate.
#[derive(Debug, Clone, PartialEq)]
struct SpritePlacement {
    offset_and_scale: [f32; 4],
    sprites: Vec<[f32; 4]>,
    animated: bool,
}

/// `round_half_away` of `k · f`, kept within the sequence.
fn sprite_index(count: usize, one_shot: bool, start: f32) -> usize {
    let k = if one_shot {
        count.saturating_sub(1)
    } else {
        count
    };
    if k == 0 {
        return 0;
    }
    ((k as f32 * start).round() as i64).rem_euclid(count as i64) as usize
}

fn sprites_of<'a>(sequence: TagStruct<'a>) -> Vec<TagStruct<'a>> {
    elements(sequence, "sprites")
}

fn sprite_placement(
    bitmap: Option<TagStruct<'_>>,
    first: usize,
    start: f32,
    one_shot: bool,
    changes: bool,
    center_offset: [f32; 2],
) -> SpritePlacement {
    let read = |holder: TagStruct<'_>| {
        ["left", "right", "top", "bottom"].map(|name| holder.read_real(name).unwrap_or(0.0))
    };
    let sequences = bitmap
        .map(|bitmap| elements(bitmap, "sequences"))
        .unwrap_or_default();
    let sequence = sequences.get(first % sequences.len().max(1)).copied();
    let animated = changes && sequence.is_some_and(|sequence| sprites_of(sequence).len() > 1);
    // The first image's aspect.
    let aspect = bitmap
        .and_then(|bitmap| elements(bitmap, "bitmaps*").first().copied())
        .map(|image| {
            (
                image.read_int_any("width").unwrap_or(1) as f32,
                image.read_int_any("height").unwrap_or(1) as f32,
            )
        })
        .map_or(1.0, |(width, height)| {
            if width != height && height > 0.0 {
                width / height
            } else {
                1.0
            }
        });
    let (mut rect, mut registration) = ([0.0, 1.0, 0.0, 1.0], [0.5, 0.5]);
    if let Some(sequence) = sequence {
        let sprites = sprites_of(sequence);
        if !sprites.is_empty() {
            let sprite = sprites[sprite_index(sprites.len(), one_shot, start)];
            rect = read(sprite);
            let point = sprite.read_point2d("registration point");
            registration = [point.x, point.y];
        }
    }
    registration[0] += center_offset[0];
    registration[1] -= center_offset[1];
    let [x0, x1, y0, y1] = rect;
    let (sx, sy) = (1.0 / (x1 - x0), 1.0 / (y1 - y0));
    let (mut ax, mut ay) = (1.0, 1.0);
    if x1 - x0 <= y1 - y0 {
        ax = (x1 - x0) * sy;
    } else {
        ay = (y1 - y0) * sx;
    }
    ax *= aspect;
    let (cx, cy) = (
        2.0 * registration[0] * sx - 1.0,
        2.0 * registration[1] * sy - 1.0,
    );
    let (left, right, bottom, top) = (
        (-1.0 - cx) * ax,
        (1.0 - cx) * ax,
        (-1.0 - cy) * ay,
        (1.0 - cy) * ay,
    );
    // The frame list: the manual sequences first, at `first + 1`.
    let manual = bitmap
        .map(|bitmap| elements(bitmap, "manual_sequences"))
        .unwrap_or_default();
    let listed = if !manual.is_empty() {
        manual.get((first + 1) % manual.len()).copied()
    } else if !sequences.is_empty() {
        sequences.get((first + 1) % sequences.len()).copied()
    } else {
        None
    };
    let mut sprites: Vec<[f32; 4]> = listed
        .map(|sequence| {
            sprites_of(sequence)
                .into_iter()
                .take(15)
                .map(read)
                .map(|[x0, x1, y0, y1]| [x0, y0, x1 - x0, y1 - y0])
                .collect()
        })
        .unwrap_or_default();
    if sprites.is_empty() {
        sprites.push([x0, y0, x1 - x0, y1 - y0]);
    }
    SpritePlacement {
        offset_and_scale: [left, bottom, right - left, top - bottom],
        sprites,
        animated,
    }
}

/// `SetMaterialRenderState`'s custom-vertex sets from the bitmap's tight
/// bounds: twenty points across the five sets, or one set of four for all.
fn custom_verts(bitmap: TagStruct<'_>) -> [[[f32; 2]; 4]; 5] {
    let flags = bitmap
        .read_int_any("Flags")
        .or_else(|| bitmap.read_int_any("flags"))
        .unwrap_or(0);
    let points: Vec<[f32; 2]> = elements(bitmap, "tight bounds")
        .into_iter()
        .map(|point| {
            let uv = point.read_point2d("uv");
            [uv.x, uv.y]
        })
        .collect();
    if flags & BLACK_POINT_TIGHT_BOUNDS != 0 && points.len() == 20 {
        return std::array::from_fn(|set| std::array::from_fn(|corner| points[set * 4 + corner]));
    }
    if flags & BLACK_POINT_TIGHT_BOUNDS == 0 && flags & TIGHT_BOUNDS != 0 && points.len() == 4 {
        return [std::array::from_fn(|corner| points[corner]); 5];
    }
    [UNIT_QUAD; 5]
}

/// One particle as the update shader leaves it (`s_particle_state`), the
/// words of `pack_particle_state` to come.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct H4ParticleState {
    /// In emitter space.
    pub(crate) position: [f32; 3],
    /// Half the quad's extent.
    pub(crate) size: f32,
    pub(crate) velocity: [f32; 3],
    pub(crate) aspect: f32,
    pub(crate) random: [f32; 4],
    pub(crate) random2: [f32; 4],
    pub(crate) physical_rotation: f32,
    pub(crate) manual_rotation: f32,
    pub(crate) animated_frame: f32,
    pub(crate) manual_frame: f32,
    pub(crate) birth_time: f32,
    pub(crate) inverse_lifespan: f32,
    /// Of its life, 0 to 1.
    pub(crate) age: f32,
    pub(crate) intensity: f32,
    pub(crate) rotation_rate: f32,
    pub(crate) frame_rate: f32,
    pub(crate) simulation: [f32; 2],
    pub(crate) black_point: f32,
    pub(crate) white_point: f32,
    pub(crate) palette: f32,
    pub(crate) axis: [f32; 3],
    /// Tint times colour, and alpha.
    pub(crate) color: [f32; 4],
    /// The spawn colour, and the exposure exponent in `.3`.
    pub(crate) initial_color: [f32; 4],
}

/// f32 to f16, rounding to nearest even (HLSL `f32tof16` [inferred]).
pub(crate) fn f32_to_f16(value: f32) -> u32 {
    let bits = value.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7F_FFFF;
    if exponent == 0xFF {
        return sign | 0x7C00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1F {
        return sign | 0x7C00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let full = mantissa | 0x80_0000;
        let shift = (14 - half_exponent) as u32;
        let half = full >> shift;
        let rest = full & ((1 << shift) - 1);
        let middle = 1 << (shift - 1);
        let rounded = if rest > middle || (rest == middle && half & 1 != 0) {
            half + 1
        } else {
            half
        };
        return sign | rounded;
    }
    let half = ((half_exponent as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1FFF;
    let rounded = if rest > 0x1000 || (rest == 0x1000 && half & 1 != 0) {
        half + 1
    } else {
        half
    };
    sign | rounded
}

fn pack_half2(x: f32, y: f32) -> u32 {
    f32_to_f16(x) | (f32_to_f16(y) << 16)
}

fn pack_ushort2n(x: f32, y: f32) -> u32 {
    let unit = |value: f32| (value.clamp(0.0, 1.0) * 65535.0) as u32;
    unit(x) | (unit(y) << 16)
}

fn pack_dec3n(x: f32, y: f32, z: f32) -> u32 {
    let signed = |value: f32| ((value.clamp(-1.0, 1.0) * 511.0) as i32 as u32) & 0x3FF;
    signed(x) | (signed(y) << 10) | (signed(z) << 20)
}

fn pack_argb8([r, g, b, a]: [f32; 4]) -> u32 {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0) as u32;
    byte(b) | (byte(g) << 8) | (byte(r) << 16) | (byte(a) << 24)
}

/// `pack_particle_state`: the twenty words of `s_raw_particle_state`.
pub(crate) fn pack_particle_state(state: &H4ParticleState) -> [u32; 20] {
    let [x, y, z] = state.position.map(f32::to_bits);
    let [vx, vy, vz] = state.velocity;
    [
        x,
        y,
        z,
        state.size.to_bits(),
        pack_half2(vx, vy),
        pack_half2(vz, state.aspect),
        pack_ushort2n(state.random[0], state.random[1]),
        pack_ushort2n(state.random[2], state.random[3]),
        pack_ushort2n(state.random2[0], state.random2[1]),
        pack_ushort2n(state.random2[2], state.random2[3]),
        pack_ushort2n(state.physical_rotation, state.manual_rotation),
        pack_ushort2n(state.animated_frame, state.manual_frame),
        pack_half2(state.birth_time, state.inverse_lifespan),
        pack_half2(state.age, state.intensity),
        pack_half2(state.rotation_rate, state.frame_rate),
        pack_half2(state.simulation[0], state.simulation[1]),
        pack_dec3n(state.black_point, state.white_point, state.palette),
        pack_dec3n(state.axis[0], state.axis[1], state.axis[2]),
        pack_argb8(state.color),
        pack_argb8(state.initial_color),
    ]
}

/// The `customVerts` registers: two corners to a `float4`, ten of them.
pub(crate) fn custom_vert_registers(sets: &[[[f32; 2]; 4]; 5]) -> [[f32; 4]; 10] {
    std::array::from_fn(|register| {
        let set = &sets[register / 2];
        let [a, b] = if register % 2 == 0 {
            [set[0], set[1]]
        } else {
            [set[2], set[3]]
        };
        [a[0], a[1], b[0], b[1]]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_to_nearest_even_and_keep_their_extremes() {
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(0.5), 0x3800);
        assert_eq!(f32_to_f16(-2.0), 0xC000);
        assert_eq!(f32_to_f16(65504.0), 0x7BFF);
        assert_eq!(f32_to_f16(1e6), 0x7C00, "past the largest half is infinite");
        assert_eq!(f32_to_f16(0.0), 0);
        // 1 + 2^-11 lies halfway between 1 and the next half: even wins.
        assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3C00);
        assert_eq!(f32_to_f16(1.0 + 3.0 * 2f32.powi(-11)), 0x3C02);
        assert_eq!(f32_to_f16(2f32.powi(-24)), 1, "the smallest subnormal");
    }

    /// The equipment icon's words at rest (`particles.md` §5): black and
    /// white points (0, 1, 0) → 0x0007FC00, white with alpha → 0xFFFFFFFF,
    /// the spawn colour white with exponent 0 → 0x00FFFFFF.
    #[test]
    fn the_equipment_icon_packs_as_the_report_has_it() {
        let state = H4ParticleState {
            size: 0.115,
            aspect: 1.0,
            inverse_lifespan: 0.5,
            age: 0.25,
            intensity: 1.0,
            white_point: 1.0,
            color: [1.0; 4],
            initial_color: [1.0, 1.0, 1.0, 0.0],
            ..Default::default()
        };
        let words = pack_particle_state(&state);
        assert_eq!(words[3], 0.115f32.to_bits());
        assert_eq!(words[5] >> 16, 0x3C00, "aspect 1 in the high half");
        assert_eq!(
            words[13],
            0x3400 | (0x3C00 << 16),
            "age a quarter, intensity 1"
        );
        assert_eq!(words[16], 0x0007_FC00);
        assert_eq!(words[18], 0xFFFF_FFFF);
        assert_eq!(words[19], 0x00FF_FFFF);
    }

    /// Every cast truncates: a black point of 0.2 packs to 102/511, which picks
    /// set 0, not 1; an alpha under 1/255 packs to nothing.
    #[test]
    fn packing_truncates() {
        let black = pack_dec3n(0.2, 0.0, 0.0) & 0x3FF;
        assert_eq!(black, 102);
        assert_eq!(((black as f32 / 511.0) * 5.0) as i32, 0);
        assert_eq!(pack_argb8([0.0, 0.0, 0.0, 0.003]) >> 24, 0);
        assert_eq!(pack_dec3n(-1.0, 0.0, 0.0) & 0x3FF, (-511i32 as u32) & 0x3FF);
        assert_eq!(pack_ushort2n(1.0, 0.5), 0xFFFF | (32767 << 16));
    }

    #[test]
    fn a_square_untouched_sprite_spans_minus_one_to_one() {
        let placement = sprite_placement(None, 0, 0.0, false, false, [0.0; 2]);
        assert_eq!(placement.offset_and_scale, [-1.0, -1.0, 2.0, 2.0]);
        assert_eq!(placement.sprites, vec![[0.0, 0.0, 1.0, 1.0]]);
        assert!(!placement.animated);
        // A centre offset moves the registration, so the quad shifts.
        let moved = sprite_placement(None, 0, 0.0, false, false, [0.25, 0.0]);
        assert_eq!(moved.offset_and_scale, [-1.5, -1.0, 2.0, 2.0]);
    }

    #[test]
    fn custom_vert_registers_hold_two_corners_each() {
        let registers = custom_vert_registers(&[UNIT_QUAD; 5]);
        assert_eq!(registers[0], [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(registers[1], [1.0, 1.0, 0.0, 1.0]);
        assert_eq!(registers[9], [1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn sprite_index_rounds_half_away_and_wraps() {
        assert_eq!(sprite_index(4, false, 0.0), 0);
        assert_eq!(
            sprite_index(4, false, 0.125),
            1,
            "0.5 rounds away from zero"
        );
        assert_eq!(sprite_index(4, false, 1.0), 0, "wraps");
        assert_eq!(sprite_index(4, true, 1.0), 3, "one shot stops at the last");
    }

    /// The sprint pickup's icon (`particles.md` §5): no attachments (main
    /// 0x40), depth fade alone among the runtime bits, a static frame, the
    /// whole sprite, `add_src_times_srcalpha`, and the unit quad for every
    /// custom set, as its bitmap has no tight bounds.
    #[test]
    fn the_equipment_icon_reads_its_draw() {
        let root = std::path::PathBuf::from(crate::core::test_kits::tag_path("halo4_mcc", ""));
        if !root.join("objects").is_dir() {
            eprintln!("skipped: set BLAM_TEST_H4EK to a Halo 4 kit's tags folder");
            return;
        }
        let source = TagSource::LooseFolder {
            root,
            game: Some(crate::core::game::GameId::Halo4),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        };
        let tag = load_referenced_tag_from_source(
            &source,
            "objects\\equipment\\sprint\\fx\\particles\\equipment_icon",
            "particle",
            b"prt3",
        )
        .unwrap();
        let render = read_h4_particle_render(&source, tag.root(), false).unwrap();
        assert_eq!(render.flags, [0x40, DEPTH_FADE, STATIC_FRAME]);
        assert_eq!(render.material.blend_mode, BLEND_ADD_SRC_TIMES_SRCALPHA);
        assert_eq!(render.sprite, [-1.0, -1.0, 2.0, 2.0]);
        assert_eq!(render.sprites, vec![[0.0, 0.0, 1.0, 1.0]]);
        assert_eq!(render.custom_verts, [UNIT_QUAD; 5]);
        assert_eq!(render.billboard, 0);
        assert!(
            (render.edge_fade[0] - 2.291831).abs() < 1e-4,
            "{:?}",
            render.edge_fade
        );
        assert!(
            render
                .material
                .textures
                .iter()
                .any(|texture| texture.uniform == "basemapTexture" && texture.frames.len() == 1)
        );
        assert_eq!(render.appearance() & LIGHTMAP_LIT, 0);
    }
}
