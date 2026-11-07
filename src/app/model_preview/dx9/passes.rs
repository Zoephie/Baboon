//! The dx9 pass model Halo CE and Halo 2 draw through: the layers in the
//! game's order, the default and shadow lights, a pass's blend, depth,
//! textures and animated constants, the draw pass the renderer binds, and the
//! objects' change colors. Halo 2's templates are read into it in
//! `h2::passes`; Halo CE's shaders in `ce::passes`.

use super::super::common::named_function::tag_function_at;
use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};
use blam_tags::tag_function::FunctionType;

use super::super::functions::FunctionValues;

use super::super::h2::prt::H2PrtLight;
use super::super::ce::passes::{CeModelAnimation, CeNumeric};
use super::stage::{H2_TEXTURE_ROWS, H2VertexStage};
use super::super::common::image::H2Image;
use crate::core::h2_pixel_shader::{
    AlphaTest, SamplerKind,
};

/// A layer the preview draws, in the order the game's `render_scene` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H2Layer {
    Texaccum,
    /// A late fill of the texaccum target, drawn to it per light in the game,
    /// for shaders whose color changes rather than sitting in a base map.
    LightAlbedo,
    /// The fallback for a scene with no lightmaps, which adds the default
    /// lights on top.
    Lightmap,
    /// The object path in a level with lightmaps; see `H2_OBJECT_LIGHTMAP_IMPLEMENTATION`.
    ObjectLightmap,
    /// Precomputed radiance transfer, for the shaders with no lightmap pass.
    Prt,
    Environment,
    SelfIllumination,
    Overlay,
    LightDiffuse,
    /// `LightDiffuse` for a light of another `diffuse contrast`: `high`.
    LightDiffuseHigh,
    /// `low`.
    LightDiffuseLow,
    /// `very low`.
    LightDiffuseVeryLow,
    /// A frustum light's gel pass, drawn once per light.
    LightGel,
    /// A frustum light with specular: its diffuse pass's first implementation.
    LightFrustum,
    /// What `shadow_apply_combined`'s third implementation leaves in the frame's
    /// alpha for `LightFrustum`.
    GelMask,
    LightSpecular,
    Hologram,
    /// Takes over from every other opaque layer while camo is on.
    ActiveCamo,
    Transparent,
    /// A model's `shield damaged shader`, over all of it.
    ShieldDamaged,
    /// A model's `overshield shader`, over all of it.
    Overshield,
    /// A model's `hologram shader`'s bloom passes, over all of it while its
    /// `hologram control function` is above nothing.
    HologramBloom,
    /// A mask written to the frame's alpha, for the glow drawn from it.
    Bloom,
    /// Not the game's: see `h2_shadow_passes`.
    ShadowCaster,
    ShadowMask,
    /// Halo 3's `shadow_apply`, darkening what the lit pass drew.
    ShadowApply,
}

pub(crate) const H2_LAYER_COUNT: usize = 26;

/// The sphere lights `render_lights` falls back on with nothing else lighting
/// the scene: a warm key and a cool fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2DefaultLight {
    pub(crate) position: [f32; 3],
    pub(crate) color: [f32; 3],
    pub(crate) specular: [f32; 3],
    pub(crate) radius: f32,
}

pub(crate) const H2_DEFAULT_LIGHTS: [H2DefaultLight; 2] = [
    H2DefaultLight {
        position: [2000.0, 2000.0, 2000.0],
        color: [0.9, 0.9, 0.8],
        specular: [0.9, 0.9, 0.8],
        radius: 40000.0,
    },
    H2DefaultLight {
        position: [-2000.0, -2000.0, 500.0],
        color: [0.2, 0.2, 0.3],
        specular: [0.2, 0.2, 0.3],
        radius: 40000.0,
    },
];

impl H2Layer {
    pub(crate) const ALL: [Self; H2_LAYER_COUNT] = [
        Self::Texaccum,
        Self::LightAlbedo,
        Self::Lightmap,
        Self::ObjectLightmap,
        Self::Prt,
        Self::Environment,
        Self::SelfIllumination,
        Self::Overlay,
        Self::LightDiffuse,
        Self::LightDiffuseHigh,
        Self::LightDiffuseLow,
        Self::LightDiffuseVeryLow,
        Self::LightGel,
        Self::LightFrustum,
        Self::GelMask,
        Self::LightSpecular,
        Self::Hologram,
        Self::ActiveCamo,
        Self::Transparent,
        Self::ShieldDamaged,
        Self::Overshield,
        Self::HologramBloom,
        Self::Bloom,
        Self::ShadowCaster,
        Self::ShadowMask,
        Self::ShadowApply,
    ];

    /// The tag's `layer` option.
    pub(in crate::app) fn tag_name(self) -> &'static str {
        match self {
            Self::Texaccum => TEXACCUM_LAYER,
            Self::LightAlbedo => "light albedo",
            Self::Lightmap | Self::ObjectLightmap => LIGHTMAP_LAYER,
            Self::Prt => "sh prt",
            Self::Environment => "environment map",
            Self::SelfIllumination => "self-illumination",
            Self::Overlay => "overlay",
            Self::LightDiffuse
            | Self::LightDiffuseHigh
            | Self::LightDiffuseLow
            | Self::LightDiffuseVeryLow
            | Self::LightGel
            | Self::LightFrustum => "diffuse",
            Self::LightSpecular => "specular",
            Self::Hologram => "hologram",
            Self::ActiveCamo => "active camo",
            Self::Bloom => "bloom",
            Self::GelMask | Self::ShadowCaster | Self::ShadowMask | Self::ShadowApply | Self::ShieldDamaged | Self::Overshield | Self::HologramBloom => "",
            Self::Transparent => TRANSPARENT_LAYER,
        }
    }

    /// The layer drawing a light of this `diffuse contrast`, by its option's
    /// name, which `rasterizer_get_switch_extern` hands the pass as
    /// `light_diffuse_contrast`.
    pub(crate) fn diffuse_with_contrast(contrast: Option<&str>) -> Self {
        match contrast {
            Some("high") => Self::LightDiffuseHigh,
            Some("low") => Self::LightDiffuseLow,
            Some("very low") => Self::LightDiffuseVeryLow,
            _ => Self::LightDiffuse,
        }
    }

    /// What this layer sets `light_diffuse_contrast` to.
    pub(in crate::app) fn diffuse_contrast(self) -> i32 {
        match self {
            Self::LightDiffuseHigh => 1,
            Self::LightDiffuseLow => 2,
            Self::LightDiffuseVeryLow => 3,
            _ => 0,
        }
    }

    /// An object's implementation on this layer. Only the lightmap layer has
    /// a choice to make; see `rasterizer_shader_get_implementation_index`.
    pub(in crate::app) fn implementation(self) -> usize {
        match self {
            Self::Lightmap => H2_FALLBACK_LIGHTMAP_IMPLEMENTATION,
            Self::ObjectLightmap => H2_OBJECT_LIGHTMAP_IMPLEMENTATION,
            Self::LightDiffuse
            | Self::LightDiffuseHigh
            | Self::LightDiffuseLow
            | Self::LightDiffuseVeryLow => H2_SPHERE_LIGHT_DIFFUSE_IMPLEMENTATION,
            Self::LightGel => H2_GEL_LIGHT_DIFFUSE_IMPLEMENTATION,
            Self::ActiveCamo => H2_FULL_ACTIVE_CAMO_IMPLEMENTATION,
            _ => 0,
        }
    }
}

/// The passes behind the preview's own key light shadow. Under the default
/// lights the game draws no shadows at all: layers 10 and 11 only run for a
/// light object that casts them.
pub(crate) fn h2_shadow_passes() -> [(H2Layer, H2DrawPass); 4] {
    let pass = |stage, fragment: String, compare, write| H2DrawPass {
        stage,
        fragment,
        blend: None,
        depth_bias: 0.0,
        depth: H2Depth { compare: Some(compare), write },
        cull: H2Cull::Frame,
        hidden_by_camo: false,
        numeric: None,
        channels: [false; 4],
        textures: Vec::new(),
        constants: Vec::new(),
        texture_rows: identity_rows(),
        tint_facing: [1.0; 4],
        tint_glancing: [1.0; 4],
        change_colors: Vec::new(),
        light_colors: Vec::new(),
        camo_amounts: Vec::new(),
        light_direct: [0.0; 3],
        light_indirect: [0.0; 3],
        animated_constants: Vec::new(),
        animated_rows: Vec::new(),
        animated_row_values: Vec::new(),
        animated_tint_facing: Vec::new(),
        animated_tint_glancing: Vec::new(),
        ce_animation: None,
    };
    [
        (
            H2Layer::ShadowCaster,
            pass(
                H2VertexStage::ShadowCaster,
                format!("{H2_GLSL_VERSION}void main() {{}}\n"),
                H2Compare::LessOrEqual,
                true,
            ),
        ),
        (
            H2Layer::ShadowMask,
            pass(
                H2VertexStage::ShadowMask,
                format!(
                    "{H2_GLSL_VERSION}in vec4 v_tc0;
uniform sampler2D u_shadow_map;
void main() {{
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    if (place.z - {H2_SHADOW_BIAS:.4} <= texture(u_shadow_map, place.xy).r) {{
        discard;
    }}
}}
"
                ),
                H2Compare::Equal,
                false,
            ),
        ),
        (
            H2Layer::GelMask,
            H2DrawPass {
                channels: [false, false, false, true],
                ..pass(
                    H2VertexStage::LightGel,
                    format!(
                        "{H2_GLSL_VERSION}in vec4 v_tc3;
in vec4 v_color1;
uniform sampler2D u_gel_map;
out vec4 color;
void main() {{
    color = vec4(1.0, 1.0, 1.0, v_color1.r > 0.5 ? textureProj(u_gel_map, v_tc3).b : 0.0);
}}
"
                    ),
                    H2Compare::Equal,
                    false,
                )
            },
        ),
        (
            H2Layer::ShadowApply,
            H2DrawPass {
                depth_bias: H3_SHADOW_APPLY_BIAS,
                // The accumulation's alpha is what covers the background.
                channels: [true, true, true, false],
                blend: Some(H2Blend {
                    equation: H2BlendEquation::Add,
                    source: H2BlendFactor::Zero,
                    destination: H2BlendFactor::SrcAlpha,
                    constant: [0; 4],
                }),
                ..pass(
                    H2VertexStage::ShadowMask,
                    format!("{H2_GLSL_VERSION}{H3_SHADOW_APPLY}"),
                    // The lit pass's depth is another program's: near, not equal.
                    H2Compare::LessOrEqual,
                    false,
                )
            },
        ),
    ]
}

const H3_SHADOW_APPLY_BIAS: f32 = -0.00002;

/// `shadow_apply_bilinear`'s pixel shader in the preview's box: one object
/// filling the map. `FASTER_SHADOWS` at `DX_VERSION` 11 leaves no slope to bias
/// by, that being the shadow generate's own.
const H3_SHADOW_APPLY: &str = "in vec4 v_tc0;
uniform sampler2D u_shadow_map;
uniform sampler2D u_normal_target;
uniform vec3 u_shadow_toward;
uniform float u_shadow_size;
uniform float u_shadow_alpha;
out vec4 color;
float closer(ivec2 texel, ivec2 offset, float depth) {
    return step(depth, texelFetch(u_shadow_map, texel + offset, 0).r);
}
void main() {
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    vec3 normal = texelFetch(u_normal_target, ivec2(gl_FragCoord.xy), 0).xyz * 2.0 - 1.0;
    float cosine = dot(normal, u_shadow_toward);
    float falloff = clamp(place.z * 2.0 - 1.0, 0.0, 1.0);
    falloff *= falloff;
    float darkness = u_shadow_alpha * (1.0 - falloff * falloff) * clamp(cosine, 0.0, 1.0);
    float darken = 1.0;
    vec2 pixel = place.xy * u_shadow_size;
    if (darkness > 0.001 && all(greaterThan(pixel, vec2(1.0))) && all(lessThan(pixel, vec2(u_shadow_size - 2.0)))) {
        ivec2 t = ivec2(floor(pixel));
        vec2 f = fract(pixel);
        vec2 r = 1.0 - f;
        float lit = r.x * r.y * closer(t, ivec2(-1, -1), place.z)
            + r.y * closer(t, ivec2(0, -1), place.z)
            + f.x * r.y * closer(t, ivec2(1, -1), place.z)
            + r.x * closer(t, ivec2(-1, 0), place.z)
            + closer(t, ivec2(0, 0), place.z)
            + f.x * closer(t, ivec2(1, 0), place.z)
            + r.x * f.y * closer(t, ivec2(-1, 1), place.z)
            + f.y * closer(t, ivec2(0, 1), place.z)
            + f.x * f.y * closer(t, ivec2(1, 1), place.z);
        lit *= 0.25;
        darken = clamp(1.01 - darkness + lit * darkness, 0.0, 1.0);
        darken *= darken;
    }
    color = vec4(0.0, 0.0, 0.0, darken);
}
";

/// Stands in for the direct light on the object path, where a shadow leaves
/// the pass only its ambient, indirect and reflected light.
pub(crate) const H2_SHADOWED_LIGHT: H2DefaultLight =
    H2DefaultLight { position: [0.0; 3], color: [0.0; 3], specular: [0.0; 3], radius: 1.0 };

/// Which way the object path's direct light lies from a surface. A pass
/// lights by the negated vector.
pub(crate) fn h2_object_light_direction() -> [f32; 3] {
    let [x, y, z, _] = DEFAULT_DIRECT_VECTOR;
    let length = (x * x + y * y + z * z).sqrt();
    [-x / length, -y / length, -z / length]
}

/// The object path's lighting as a PRT pass takes it: the indirect light,
/// then the direct one from `direction`. The game samples a level's probes
/// here, which a preview stands in for with the same colors the other passes
/// light by. The indirect light alone is what a shadow leaves.
pub(crate) fn h2_prt_object_lights(direction: [f32; 3]) -> ([H2PrtLight; 2], [f32; 3]) {
    let channels = |[r, g, b, _]: [f32; 4]| [r, g, b];
    let [x, y, z, _] = DEFAULT_INDIRECT_VECTOR;
    let lights = [
        H2PrtLight { direction: [-x, -y, -z], color: channels(DEFAULT_INDIRECT_COLOR) },
        H2PrtLight { direction, color: channels(DEFAULT_DIRECT_COLOR) },
    ];
    (lights, channels(DEFAULT_AMBIENT))
}

/// Enough to clear a surface of its own shadow, as a share of the depth range.
const H2_SHADOW_BIAS: f32 = 0.004;

/// What a model's `sh prt` passes are lit by. The game samples the level's
/// light probes here, and with none it falls back on a stand-in from straight
/// below; a preview uses its own lights instead.
pub(crate) fn h2_prt_lights() -> [H2PrtLight; 2] {
    H2_DEFAULT_LIGHTS.map(|light| {
        let [x, y, z] = light.position;
        let length = (x * x + y * y + z * z).sqrt();
        H2PrtLight { direction: [x / length, y / length, z / length], color: light.color }
    })
}

const TRANSPARENT_LAYER: &str = "transparent";
const LIGHTMAP_LAYER: &str = "lightmap (indirect)";
const TEXACCUM_LAYER: &str = "texaccum";
/// What `render_visible_section_get_lightmap_mode` picks when the level has
/// no lightmaps: fixed lighting, with the default lights doing the rest.
const H2_FALLBACK_LIGHTMAP_IMPLEMENTATION: usize = 3;
/// What it picks for a model in a level with lightmaps. This pass reads the
/// object's lighting and takes its reflection from the object's brightness.
const H2_OBJECT_LIGHTMAP_IMPLEMENTATION: usize = 2;
/// "amount=1 (no blending, requires no opaque render)".
const H2_FULL_ACTIVE_CAMO_IMPLEMENTATION: usize = 1;
/// `render_light_get_diffuse_implementation_index` for a light of type sphere.
const H2_SPHERE_LIGHT_DIFFUSE_IMPLEMENTATION: usize = 1;
/// The same for a frustum with neither specular nor a shadow.
const H2_GEL_LIGHT_DIFFUSE_IMPLEMENTATION: usize = 3;
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum H2ParameterValue {
    Bitmap(String),
    Value(f32),
    Color([f32; 3]),
    /// Filled at runtime, by this option name of the pass's `source extern`.
    Extern(String),
    /// Nothing in the shader, template or pass supplies it.
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum H2BlendFactor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstColor,
    OneMinusDstColor,
    DstAlpha,
    OneMinusDstAlpha,
    SrcAlphaSaturate,
    ConstantColor,
    OneMinusConstantColor,
    ConstantAlpha,
    OneMinusConstantAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum H2BlendEquation {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app) struct H2Blend {
    pub(in crate::app) equation: H2BlendEquation,
    pub(in crate::app) source: H2BlendFactor,
    pub(in crate::app) destination: H2BlendFactor,
    /// ARGB as stored, for the `constant` factors.
    pub(in crate::app) constant: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum H2Compare {
    Never,
    Less,
    Equal,
    LessOrEqual,
    Greater,
    NotEqual,
    GreaterOrEqual,
    Always,
}

/// Which faces a pass draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum H2Cull {
    /// As the preview's Render Backfaces has it.
    Frame,
    Back,
    Front,
    /// Both sides, whatever Render Backfaces says.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct H2Depth {
    /// `None` turns the depth test off.
    pub(in crate::app) compare: Option<H2Compare>,
    pub(in crate::app) write: bool,
}

/// What one of a shader parameter's `animation properties` drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H2AnimationKind {
    ScaleUniform,
    ScaleX,
    ScaleY,
    ScaleZ,
    TranslationX,
    TranslationY,
    TranslationZ,
    Rotation,
    Value,
    Color,
}

/// A parameter function the game reads by time, as in
/// `shader_setup_evaluate_overlay_animation`.
#[derive(Debug, Clone)]
pub(crate) struct H2Animation {
    pub(in crate::app::model_preview) kind: H2AnimationKind,
    /// Seconds to a cycle. At zero the function reads the time itself.
    pub(in crate::app::model_preview) period: f32,
    /// The object function this reads instead of time, and what it reads
    /// with no such function.
    pub(in crate::app::model_preview) held_input: Option<(String, f32)>,
    /// `range name`: the object function the range reads, and what it reads
    /// with no such function. Unnamed, `object_get_function_value` gives 1.
    pub(in crate::app::model_preview) range_input: Option<(String, f32)>,
    pub(in crate::app::model_preview) function: Arc<TagFunction>,
}

impl PartialEq for H2Animation {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.period == other.period
            && self.held_input == other.held_input
            && self.range_input == other.range_input
            && self.function.to_bytes() == other.function.to_bytes()
    }
}

/// When an animation reads, and the object functions it can read from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct H2Clock<'a> {
    pub(crate) time: f32,
    pub(crate) values: Option<&'a FunctionValues>,
}

impl From<f32> for H2Clock<'_> {
    fn from(time: f32) -> Self {
        Self { time, values: None }
    }
}

impl H2Animation {
    /// `sub_915520`: the named function's value, or with no name the time
    /// over the period, wrapped unless the function is periodic. The game
    /// divides by a period of 0 as well, which leaves it NaN; the preview
    /// reads the time as it is instead.
    fn input(&self, clock: H2Clock<'_>) -> f32 {
        match &self.held_input {
            Some((name, rest)) => {
                clock.values.and_then(|values| values.get(name)).copied().unwrap_or(*rest)
            }
            None if self.period > 0.0 => {
                let input = clock.time / self.period;
                if self.function.function_type() == FunctionType::Periodic { input } else { input.rem_euclid(1.0) }
            }
            None => clock.time,
        }
    }

    fn range(&self, clock: H2Clock<'_>) -> f32 {
        match &self.range_input {
            Some((name, rest)) => clock.values.and_then(|values| values.get(name)).copied().unwrap_or(*rest),
            None => 1.0,
        }
    }

    fn scalar<'a>(&self, clock: impl Into<H2Clock<'a>>) -> f32 {
        let clock = clock.into();
        self.function.evaluate(self.input(clock), self.range(clock))
    }

    fn vector<'a>(&self, clock: impl Into<H2Clock<'a>>) -> [f32; 4] {
        let clock = clock.into();
        match self.kind {
            H2AnimationKind::Color => {
                let color = self.function.evaluate_color(self.input(clock), self.range(clock));
                [color.red, color.green, color.blue, 1.0]
            }
            _ => [self.scalar(clock); 4],
        }
    }

    /// Whether this shows: a transform always can, but a value or color may
    /// wander by less than a step of the frame.
    fn moves(&self) -> bool {
        if self.held_input.is_some() || self.function.as_constant().is_some() {
            return false;
        }
        if self.is_transform() {
            return true;
        }
        const SAMPLES: usize = 16;
        const VISIBLE: f32 = 2.0 / 255.0;
        let span = if self.period > 0.0 { self.period } else { 1.0 };
        let first = self.vector(0.0);
        (1..SAMPLES).any(|sample| {
            let vector = self.vector(span * sample as f32 / SAMPLES as f32);
            vector.iter().zip(first).any(|(value, first)| (value - first).abs() >= VISIBLE)
        })
    }

    pub(in crate::app::model_preview) fn is_transform(&self) -> bool {
        !matches!(self.kind, H2AnimationKind::Value | H2AnimationKind::Color)
    }
}

/// The last value or color of a parameter's animations, as they overwrite
/// one another in the game.
pub(in crate::app::model_preview) fn animated_vector(animations: &[H2Animation], clock: H2Clock<'_>) -> Option<[f32; 4]> {
    animations.iter().rev().find(|animation| !animation.is_transform()).map(|last| last.vector(clock))
}

/// The bytes of a `mapping_function`, as the engine's H2 function.
pub(in crate::app::model_preview) fn animation_function(property: &TagStruct<'_>) -> Option<TagFunction> {
    tag_function_at(property, "function")
}

/// How a pass holds a texture transform in one register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum H2RowForm {
    Row1,
    Row2,
    /// The last of a 3D map's three: `xyzw row3 (3D affine xform)`.
    Row3,
    SimpleXform,
    ScaleXy,
    ScaleZw,
}

/// A texture transform to rebuild each frame.
#[derive(Debug, Clone)]
pub(crate) struct H2AnimatedRow {
    pub(in crate::app::model_preview) register: usize,
    pub(in crate::app::model_preview) form: H2RowForm,
    pub(in crate::app::model_preview) scale: [f32; 3],
    pub(in crate::app::model_preview) animations: Vec<H2Animation>,
}

impl H2AnimatedRow {
    /// `sub_6D8B60`, which turns about the middle of the map. Scale reaches
    /// the turn and not the translation, which `sub_9185D0` writes straight
    /// into its register each frame.
    fn apply(&self, rows: &mut [[f32; 4]; H2_TEXTURE_ROWS], clock: H2Clock<'_>) {
        let [mut scale_x, mut scale_y, mut scale_z] = self.scale;
        let (mut move_x, mut move_y, mut move_z, mut angle) = (0.0, 0.0, 0.0, 0.0);
        for animation in &self.animations {
            let value = animation.scalar(clock);
            match animation.kind {
                H2AnimationKind::ScaleUniform => (scale_x, scale_y, scale_z) = (value, value, value),
                H2AnimationKind::ScaleX => scale_x = value,
                H2AnimationKind::ScaleY => scale_y = value,
                H2AnimationKind::ScaleZ => scale_z = value,
                H2AnimationKind::TranslationX => move_x = value,
                H2AnimationKind::TranslationY => move_y = value,
                H2AnimationKind::TranslationZ => move_z = value,
                H2AnimationKind::Rotation => angle = value,
                H2AnimationKind::Value | H2AnimationKind::Color => {}
            }
        }
        let (sin, cos): (f32, f32) = f32::sin_cos(angle);
        let offset_x = (sin + 1.0 - cos) * 0.5 * scale_x + move_x;
        let offset_y = (1.0 - cos - sin) * 0.5 * scale_y + move_y;
        let Some(row) = rows.get_mut(self.register) else {
            return;
        };
        match self.form {
            H2RowForm::Row1 => *row = [cos * scale_x, -sin * scale_x, 0.0, offset_x],
            H2RowForm::Row2 => *row = [sin * scale_y, cos * scale_y, 0.0, offset_y],
            H2RowForm::Row3 => *row = [0.0, 0.0, scale_z, move_z],
            H2RowForm::SimpleXform => *row = [cos * scale_x, cos * scale_y, offset_x, offset_y],
            H2RowForm::ScaleXy => (row[0], row[1]) = (cos * scale_x, cos * scale_y),
            H2RowForm::ScaleZw => (row[2], row[3]) = (cos * scale_x, cos * scale_y),
        }
    }
}

/// A vertex constant the pass fills from a parameter: for a bitmap, a row of
/// its texture transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct H2VertexConstant {
    pub(in crate::app) parameter: String,
    pub(in crate::app) bank: String,
    pub(in crate::app) register: i32,
    pub(in crate::app) components: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct H2PassTexture {
    pub(in crate::app) parameter: String,
    pub(in crate::app) mode: String,
    pub(in crate::app) value: H2ParameterValue,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct H2Pass {
    pub(in crate::app) path: String,
    pub(in crate::app) implementation: usize,
    pub(in crate::app) vertex_shader: String,
    /// `None` draws without blending.
    pub(in crate::app) blend: Option<H2Blend>,
    pub(in crate::app) depth: H2Depth,
    pub(in crate::app) channels: String,
    pub(in crate::app) custom_channels: Option<[bool; 4]>,
    pub(in crate::app) alpha_test: Option<AlphaTest>,
    /// One per texture stage, in stage order.
    pub(in crate::app) textures: Vec<H2PassTexture>,
    pub(in crate::app) parameters: Vec<(String, H2ParameterValue)>,
    pub(in crate::app) vertex_constants: Vec<H2VertexConstant>,
    /// What the shader animates of `parameters`, by parameter.
    pub(in crate::app) animations: std::collections::BTreeMap<String, Vec<H2Animation>>,
}

impl H2Pass {
    /// The kit's generated source for this pass's implementation.
    pub(in crate::app) fn pixel_shader_file(&self) -> String {
        format!("{}_{}.psh", self.path.replace(['\\', '/'], "_"), self.implementation)
    }
}

pub(in crate::app::model_preview) fn elements<'a>(holder: TagStruct<'a>, name: &str) -> impl Iterator<Item = TagStruct<'a>> {
    let block = holder.field_path(name).and_then(|field| field.as_block());
    (0..block.as_ref().map_or(0, |block| block.len()))
        .filter_map(move |index| block.as_ref()?.element(index))
}

pub(in crate::app::model_preview) fn rgb(parameter: TagStruct<'_>, name: &str) -> [f32; 3] {
    let color = parameter.read_rgb(name);
    [color.red, color.green, color.blue]
}

/// One texture unit of a pass. A `None` image draws the unit's placeholder.
#[derive(Debug, Clone)]
pub(crate) struct H2DrawTexture {
    pub(crate) slot: u32,
    pub(crate) kind: SamplerKind,
    pub(crate) image: Option<std::sync::Arc<H2Image>>,
    /// Bound to the generated normalization cube instead of an image.
    pub(crate) normalization: bool,
    /// Bound to the preview's texaccum target instead of an image.
    pub(crate) texaccum: bool,
    /// Bound to a copy of the frame so far instead of an image.
    pub(crate) frame_copy: bool,
    /// Bound to a white volume. A default light's radius puts a model at the
    /// middle of its falloff, which reads 1.
    pub(crate) falloff: bool,
    /// Bound for each light from its own `gel map`.
    pub(crate) gel: bool,
}

/// A pass ready for the renderer: resolved, translated and decoded on the
/// worker, with no GL state yet.
#[derive(Debug, Clone)]
pub(crate) struct H2DrawPass {
    pub(crate) stage: H2VertexStage,
    pub(crate) fragment: String,
    pub(crate) blend: Option<H2Blend>,
    pub(crate) depth: H2Depth,
    /// Added to its window depth: what `rasterizer_dx9_set_decal_zbias` sets.
    pub(crate) depth_bias: f32,
    pub(crate) cull: H2Cull,
    /// Faded out entirely on a fully camouflaged object.
    pub(crate) hidden_by_camo: bool,
    /// One digit of a Halo CE numeric display: drawn while that digit shows.
    pub(crate) numeric: Option<CeNumeric>,
    pub(crate) channels: [bool; 4],
    pub(crate) textures: Vec<H2DrawTexture>,
    /// Pixel constant registers, by register index.
    pub(crate) constants: Vec<(u32, [f32; 4])>,
    pub(crate) texture_rows: [[f32; 4]; H2_TEXTURE_ROWS],
    pub(crate) tint_facing: [f32; 4],
    pub(crate) tint_glancing: [f32; 4],
    /// Constant components filled from the object's change colors at draw
    /// time: register, component, channel.
    pub(crate) change_colors: Vec<(u32, usize, usize)>,
    /// Constant components filled from the current light: register, component.
    pub(crate) light_colors: Vec<(u32, usize)>,
    /// Registers and components of `OBJECT active camo amount`, which holds
    /// for the object as it is drawn: plenty of passes read it with no camo on.
    pub(crate) camo_amounts: Vec<(u32, usize)>,
    /// The vectors a lightmap pass lights by. Most passes keep them in the
    /// tag, each as its pixel shader expects: some negate them.
    pub(crate) light_direct: [f32; 3],
    pub(crate) light_indirect: [f32; 3],
    /// Components of `constants` the shader animates: register, component.
    pub(in crate::app::model_preview) animated_constants: Vec<(u32, usize, Vec<H2Animation>)>,
    pub(in crate::app::model_preview) animated_rows: Vec<H2AnimatedRow>,
    /// One component of a row a stage reads as a value, and the value
    /// animations that replace it (`sub_6DE0F0`): row, component.
    pub(in crate::app::model_preview) animated_row_values: Vec<(usize, usize, Vec<H2Animation>)>,
    pub(in crate::app::model_preview) animated_tint_facing: Vec<H2AnimatedTint>,
    pub(in crate::app::model_preview) animated_tint_glancing: Vec<H2AnimatedTint>,
    /// Halo CE reworks its rows and a constant by its own functions.
    pub(crate) ce_animation: Option<std::sync::Arc<CeModelAnimation>>,
}

impl H2DrawPass {
    /// A pass with nothing bound, its stage's values and textures to fill.
    pub(in crate::app::model_preview) fn new(stage: H2VertexStage, fragment: String, depth: H2Depth) -> Self {
        Self {
            stage,
            fragment,
            blend: None,
            depth,
            depth_bias: 0.0,
            cull: H2Cull::Frame,
            hidden_by_camo: false,
            numeric: None,
            channels: [true; 4],
            textures: Vec::new(),
            constants: Vec::new(),
            texture_rows: [[0.0; 4]; H2_TEXTURE_ROWS],
            tint_facing: [1.0; 4],
            tint_glancing: [1.0; 4],
            change_colors: Vec::new(),
            light_colors: Vec::new(),
            camo_amounts: Vec::new(),
            light_direct: [0.0; 3],
            light_indirect: [0.0; 3],
            animated_constants: Vec::new(),
            animated_rows: Vec::new(),
            animated_row_values: Vec::new(),
            animated_tint_facing: Vec::new(),
            animated_tint_glancing: Vec::new(),
            ce_animation: None,
        }
    }

    /// Whether this pass looks different from one moment to the next.
    pub(crate) fn moves(&self) -> bool {
        self.animated_constants.iter().flat_map(|(_, _, animations)| animations)
            .chain(self.animated_rows.iter().flat_map(|row| &row.animations))
            .chain(self.animated_row_values.iter().flat_map(|(_, _, animations)| animations))
            .chain(self.animated_tints())
            .any(H2Animation::moves)
            || self.ce_animation.as_ref().is_some_and(|animation| animation.moves())
    }

    /// Whether it moves by itself while `outputs` hold.
    pub(crate) fn moves_with(&self, outputs: &[f32; 4]) -> bool {
        self.ce_animation.as_ref().is_some_and(|animation| animation.moves_with(outputs))
    }

    /// The object functions its animations read.
    pub(crate) fn function_inputs(&self) -> impl Iterator<Item = &str> {
        let constants = self.animated_constants.iter().flat_map(|(_, _, animations)| animations);
        constants
            .chain(self.animated_rows.iter().flat_map(|row| &row.animations))
            .chain(self.animated_row_values.iter().flat_map(|(_, _, animations)| animations))
            .chain(self.animated_tints())
            .flat_map(|animation| [&animation.held_input, &animation.range_input])
            .filter_map(|input| input.as_ref().map(|(name, _)| name.as_str()))
    }

    /// Whether it changes with the object's outputs.
    pub(crate) fn reads_outputs(&self) -> bool {
        self.numeric.is_some()
            || self.ce_animation.as_ref().is_some_and(|animation| animation.reads_outputs())
    }

    pub(crate) fn texture_rows_at(&self, clock: H2Clock<'_>, outputs: &[f32; 4]) -> [[f32; 4]; H2_TEXTURE_ROWS] {
        let mut rows = self.texture_rows;
        self.animated_rows.iter().for_each(|row| row.apply(&mut rows, clock));
        for (row, channel, animations) in &self.animated_row_values {
            if let Some(vector) = animated_vector(animations, clock) {
                rows[*row][*channel] = vector[*channel];
            }
        }
        if let Some(animation) = &self.ce_animation {
            animation.apply_rows(&mut rows, clock.time, outputs);
        }
        rows
    }

    fn animated_tints(&self) -> impl Iterator<Item = &H2Animation> {
        self.animated_tint_facing.iter().chain(&self.animated_tint_glancing)
            .flat_map(|(_, animations)| animations)
    }

    pub(crate) fn tints_at(&self, clock: H2Clock<'_>) -> ([f32; 4], [f32; 4]) {
        let at = |mut tint: [f32; 4], animated: &[H2AnimatedTint]| {
            for (mask, animations) in animated {
                if let Some(vector) = animated_vector(animations, clock) {
                    fill_components(&mut tint, vector, *mask);
                }
            }
            tint
        };
        (
            at(self.tint_facing, &self.animated_tint_facing),
            at(self.tint_glancing, &self.animated_tint_glancing),
        )
    }

    /// Fill what the shader animates of `value`, the constant at `register`.
    pub(crate) fn animate_constant(&self, register: u32, value: &mut [f32; 4], clock: H2Clock<'_>) {
        for (animated, component, animations) in &self.animated_constants {
            if *animated == register
                && let Some(vector) = animated_vector(animations, clock)
            {
                value[*component] = vector[*component];
            }
        }
    }
}

/// One initial permutation of a change color channel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H2ChangePermutation {
    pub(crate) weight: f32,
    pub(crate) lower: [f32; 3],
    pub(crate) upper: [f32; 3],
    pub(crate) variant: String,
}

/// Animations of a tint, with the components they write.
type H2AnimatedTint = ([bool; 4], Vec<H2Animation>);

/// An object's four change color channels.
pub(crate) type H2ChangeColors = [Vec<H2ChangePermutation>; 4];

/// The object's change colors. Halo 2 nests each group's parent as a struct,
/// so `object` sits at the end of the chain of leading structs.
pub(in crate::app) fn read_h2_change_colors(object: &TagFile) -> H2ChangeColors {
    let mut holder = object.root();
    let block = loop {
        if holder.field("change colors").is_some() {
            break Some(holder);
        }
        let Some(parent) = holder.fields_all().find_map(|field| field.as_struct()) else {
            break None;
        };
        holder = parent;
    };
    let mut colors = H2ChangeColors::default();
    let Some(block) = block else {
        return colors;
    };
    for (channel, entry) in elements(block, "change colors").take(4).enumerate() {
        // Halo CE's block has another name, and no variants.
        let permutations = ["initial permutations", "permutations"]
            .into_iter()
            .find(|name| entry.field(name).is_some())
            .unwrap_or("initial permutations");
        colors[channel] = elements(entry, permutations)
            .map(|permutation| H2ChangePermutation {
                weight: permutation.read_real("weight").unwrap_or(0.0),
                lower: rgb(permutation, "color lower bound"),
                upper: rgb(permutation, "color upper bound"),
                variant: permutation.read_string_id("variant name").unwrap_or_default(),
            })
            .collect();
    }
    colors
}

/// One channel's color for a model variant. The game picks by a per-object
/// position; a preview has none, so this takes the permutation at the middle
/// of the weights and the middle of its range. White with nothing allowed, as
/// the game does.
fn change_color_for(permutations: &[H2ChangePermutation], variant: &str) -> [f32; 3] {
    // `object_postprocess` shares out a block with no weights evenly, so
    // there a zero is as likely as any other entry and not ruled out.
    let unweighted = permutations.iter().all(|entry| entry.weight <= 0.0);
    let weight = |entry: &H2ChangePermutation| if unweighted { 1.0 } else { entry.weight };
    let allowed: Vec<_> = permutations
        .iter()
        .filter(|entry| weight(entry) > 0.0)
        .filter(|entry| entry.variant.is_empty() || entry.variant == variant)
        .collect();
    let total: f32 = allowed.iter().map(|entry| weight(entry)).sum();
    let mut remaining = total / 2.0;
    for entry in allowed {
        remaining -= weight(entry);
        if remaining <= 0.0 {
            return std::array::from_fn(|axis| (entry.lower[axis] + entry.upper[axis]) / 2.0);
        }
    }
    [1.0; 3]
}

pub(crate) fn h2_change_colors_for(colors: &H2ChangeColors, variant: &str) -> [[f32; 3]; 4] {
    std::array::from_fn(|channel| change_color_for(&colors[channel], variant))
}

fn rgb_to_hsv([red, green, blue]: [f32; 3]) -> [f32; 3] {
    let high = red.max(green).max(blue);
    let span = high - red.min(green).min(blue);
    let hue = if span == 0.0 {
        0.0
    } else if high == red {
        ((green - blue) / span).rem_euclid(6.0)
    } else if high == green {
        (blue - red) / span + 2.0
    } else {
        (red - green) / span + 4.0
    };
    [hue / 6.0, if high == 0.0 { 0.0 } else { span / high }, high]
}

fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> [f32; 3] {
    let sector = hue.rem_euclid(1.0) * 6.0;
    let whole = sector.floor();
    let part = sector - whole;
    let (low, falling, rising) =
        (value * (1.0 - saturation), value * (1.0 - saturation * part), value * (1.0 - saturation * (1.0 - part)));
    match whole as u8 {
        0 => [value, rising, low],
        1 => [falling, value, low],
        2 => [low, value, rising],
        3 => [low, falling, value],
        4 => [rising, low, value],
        _ => [value, low, falling],
    }
}

/// `rgb_colors_interpolate` with its HSV flag: the short way round the hue,
/// or the long way with its `...more colors` flag.
fn hsv_interpolate(lower: [f32; 3], upper: [f32; 3], u: f32, long_way: bool) -> [f32; 3] {
    let (mut from, mut to) = (rgb_to_hsv(lower), rgb_to_hsv(upper));
    if ((from[0] - to[0]).abs() > 0.5) != long_way {
        if from[0] >= to[0] {
            to[0] += 1.0;
        } else {
            from[0] += 1.0;
        }
    }
    hsv_to_rgb(std::array::from_fn(|channel| from[channel] * (1.0 - u) + to[channel] * u))
}

/// `object_choose_random_change_colors` for an object at the world's origin,
/// where its hash of the position leaves only the channel. White with no
/// permutation, as an unplaced object has.
fn ce_change_color(permutations: &[H2ChangePermutation], channel: usize) -> [f32; 3] {
    let pick = (channel as f32 * 431.12894).rem_euclid(1.0);
    let u = (channel as f32 * 0.71211).rem_euclid(1.0);
    // Tool bakes the weights to a running share of the whole, shared out
    // evenly where a block has none.
    let unweighted = permutations.iter().all(|entry| entry.weight <= 0.0);
    let weight = |entry: &H2ChangePermutation| if unweighted { 1.0 } else { entry.weight };
    let total: f32 = permutations.iter().map(weight).sum();
    let mut running = 0.0;
    for entry in permutations {
        running += weight(entry) / total;
        if pick <= running {
            return hsv_interpolate(entry.lower, entry.upper, u, false).map(|value| value.clamp(0.0, 1.0));
        }
    }
    [1.0; 3]
}

/// `rgb_colors_interpolate`'s flags from a tag's two options: whether to
/// blend in HSV, and whether the long way round the hue wheel.
pub(crate) fn ce_color_interpolation(hsv: bool, long_way: bool) -> i128 {
    i128::from(hsv) | i128::from(long_way) << 1
}

/// `rgb_colors_interpolate`: bit 0 of `flags` blends in HSV, bit 1 the long way.
pub(crate) fn ce_color_interpolate(flags: i128, lower: [f32; 3], upper: [f32; 3], u: f32) -> [f32; 3] {
    if flags & 1 != 0 {
        hsv_interpolate(lower, upper, u, flags & 2 != 0)
    } else {
        std::array::from_fn(|channel| lower[channel] * (1.0 - u) + upper[channel] * u)
    }
}

pub(crate) fn ce_change_colors(colors: &H2ChangeColors) -> [[f32; 3]; 4] {
    std::array::from_fn(|channel| ce_change_color(&colors[channel], channel))
}

/// One face of the cube that normalizes a vector by sampling it, in the GL
/// face order: each texel holds its own direction, packed to 0..1.
pub(crate) fn h2_normalization_face(face: usize, size: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let direction = match face {
                0 => [1.0, -v, -u],
                1 => [-1.0, -v, u],
                2 => [u, 1.0, v],
                3 => [u, -1.0, -v],
                4 => [u, -v, 1.0],
                _ => [-u, -v, -1.0],
            };
            let length = direction.iter().map(|axis: &f32| axis * axis).sum::<f32>().sqrt();
            for axis in direction {
                rgba.push(((axis / length * 0.5 + 0.5) * 255.0).round() as u8);
            }
            rgba.push(255);
        }
    }
    rgba
}

/// The GL version every translated shader and vertex stage declares.
pub(crate) const H2_GLSL_VERSION: &str = "#version 330\n";

/// What a preview has in place of a level's lighting: the colors the
/// no-lightmap passes hold as literals, so both paths light a surface alike.
/// The vectors are the game's `default_object_lighting`.
pub(in crate::app::model_preview) const DEFAULT_AMBIENT: [f32; 4] = [0.0627, 0.0627, 0.0627, 1.0];
pub(in crate::app::model_preview) const DEFAULT_DIRECT_COLOR: [f32; 4] = [0.502, 0.490, 0.471, 1.0];
pub(in crate::app::model_preview) const DEFAULT_DIRECT_VECTOR: [f32; 4] = [-0.577, -0.577, -0.577, 1.0];
pub(in crate::app::model_preview) const DEFAULT_INDIRECT_COLOR: [f32; 4] = [0.373, 0.353, 0.333, 1.0];
pub(in crate::app::model_preview) const DEFAULT_INDIRECT_VECTOR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
/// Reflection vectors in the game carry a scale of about 2.
pub(crate) const H2_REFLECTION_SCALE: f32 = 2.0;

pub(in crate::app::model_preview) fn fill_components(target: &mut [f32; 4], value: [f32; 4], mask: [bool; 4]) {
    for (index, write) in mask.into_iter().enumerate() {
        if write {
            target[index] = value[index];
        }
    }
}

/// Rows of an identity 2D transform: `x` in even registers, `y` in odd ones.
pub(in crate::app::model_preview) fn identity_rows() -> [[f32; 4]; H2_TEXTURE_ROWS] {
    std::array::from_fn(|row| if row % 2 == 0 { [1.0, 0.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0, 0.0] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::functions::object_input;

    fn animation(kind: H2AnimationKind, period: f32, value: f32) -> H2Animation {
        let function = blam_tags::tag_function::H2Function::new_constant(value);
        H2Animation {
            kind,
            period,
            held_input: None,
            range_input: None,
            function: Arc::new(TagFunction::parse_encoded(
                blam_tags::tag_function::FunctionEncoding::H2,
                &function.to_bytes(),
            ).unwrap()),
        }
    }

    #[test]
    fn a_period_wraps_time_and_no_period_reads_it_raw() {
        let cycling = animation(H2AnimationKind::Value, 4.0, 0.0);
        assert_eq!(cycling.input(1.0.into()), 0.25);
        assert_eq!(cycling.input(5.0.into()), 0.25);
        assert_eq!(animation(H2AnimationKind::Value, 0.0, 0.0).input(5.0.into()), 5.0);
    }

    #[test]
    fn a_texture_transform_moves_scales_and_turns_as_the_game_builds_it() {
        let row = |form, animations| {
            let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
            H2AnimatedRow { register: 1, form, scale: [2.0, 4.0, 8.0], animations }
                .apply(&mut rows, 0.0.into());
            rows[1]
        };
        let moved = vec![
            animation(H2AnimationKind::TranslationX, 0.0, 0.25),
            animation(H2AnimationKind::TranslationY, 0.0, 0.5),
        ];
        // `sub_6D8B60` leaves a translation unscaled.
        assert_eq!(row(H2RowForm::SimpleXform, moved.clone()), [2.0, 4.0, 0.25, 0.5]);
        assert_eq!(row(H2RowForm::Row1, moved.clone()), [2.0, 0.0, 0.0, 0.25]);
        assert_eq!(row(H2RowForm::Row2, moved), [0.0, 4.0, 0.0, 0.5]);

        let deep = vec![animation(H2AnimationKind::TranslationZ, 0.0, 0.5)];
        assert_eq!(row(H2RowForm::Row3, deep), [0.0, 0.0, 8.0, 0.5]);
        let even = vec![animation(H2AnimationKind::ScaleUniform, 0.0, 3.0)];
        assert_eq!(row(H2RowForm::Row3, even), [0.0, 0.0, 3.0, 0.0], "uniform scale reaches z");

        let turned = row(
            H2RowForm::Row1,
            vec![animation(H2AnimationKind::Rotation, 0.0, std::f32::consts::FRAC_PI_2)],
        );
        // A quarter turn about the middle of the map keeps the middle in place.
        assert!(turned[0].abs() < 1e-6, "{turned:?}");
        assert!((turned[1] + 2.0).abs() < 1e-6, "{turned:?}");
        assert!((turned[3] - 2.0).abs() < 1e-6, "{turned:?}");
    }

    #[test]
    fn an_object_input_holds_still_at_what_a_whole_object_exports() {
        assert_eq!(object_input("shield_vitality"), 1.0);
        assert_eq!(object_input("object_overshield_amount"), 0.0);
        assert_eq!(object_input("boost"), 0.0);
        let held = H2Animation { held_input: Some(("boost".to_owned(), 1.0)), ..animation(H2AnimationKind::Value, 4.0, 0.0) };
        assert_eq!(held.input(3.0.into()), 1.0);
        assert!(!held.moves());
        // An object's functions take over from the resting value.
        let values = FunctionValues::from([("boost".to_owned(), 0.25)]);
        assert_eq!(held.input(H2Clock { time: 3.0, values: Some(&values) }), 0.25);
    }

    #[test]
    fn only_a_non_constant_function_moves() {
        assert!(!animation(H2AnimationKind::Value, 1.0, 0.5).moves());
    }

    #[test]
    fn the_lights_choose_between_the_game_s_two_lightmap_paths() {
        assert_eq!(H2Layer::Lightmap.implementation(), 3);
        assert_eq!(H2Layer::ObjectLightmap.implementation(), 2);
        assert_eq!(H2Layer::ObjectLightmap.tag_name(), H2Layer::Lightmap.tag_name());
    }

    #[test]
    fn change_colors_filter_by_variant_and_fall_back_to_white() {
        let entry = |weight, value: f32, variant: &str| H2ChangePermutation {
            weight,
            lower: [value; 3],
            upper: [value + 0.25; 3],
            variant: variant.to_owned(),
        };
        let list = [entry(1.0, 0.25, "minor"), entry(1.0, 0.5, "major"), entry(0.0, 0.75, "")];
        assert_eq!(change_color_for(&list, "minor"), [0.375; 3]);
        assert_eq!(change_color_for(&list, "major"), [0.625; 3]);
        assert_eq!(change_color_for(&list, "ultra"), [1.0; 3], "nothing allowed");
        assert_eq!(change_color_for(&[], "minor"), [1.0; 3]);
        let shared = [entry(1.0, 0.0, ""), entry(2.0, 0.5, ""), entry(1.0, 0.75, "")];
        assert_eq!(change_color_for(&shared, "any"), [0.625; 3], "the middle of the weights");
    }

    #[test]
    fn a_block_with_no_weights_shares_them_evenly() {
        let entry = |value: f32, variant: &str| H2ChangePermutation {
            weight: 0.0,
            lower: [value; 3],
            upper: [value; 3],
            variant: variant.to_owned(),
        };
        let list = [entry(0.25, "minor"), entry(0.5, "major"), entry(0.75, "major")];
        assert_eq!(change_color_for(&list, "minor"), [0.25; 3]);
        assert_eq!(change_color_for(&list, "major"), [0.5; 3]);
        assert_eq!(change_color_for(&list, "ultra"), [1.0; 3], "no entry for this variant");
    }

    #[test]
    fn normalization_faces_hold_their_own_direction() {
        let center = |face: usize, size: usize| -> [u8; 3] {
            let rgba = h2_normalization_face(face, size);
            let at = (size / 2 * size + size / 2) * 4;
            [rgba[at], rgba[at + 1], rgba[at + 2]]
        };
        assert!(center(0, 33)[0] > 250, "+X");
        assert!(center(3, 33)[1] < 5, "-Y");
        assert!(center(4, 33)[2] > 250, "+Z");
        assert!(center(5, 33)[2] < 5, "-Z");
    }

    #[test]
    fn the_source_file_follows_the_pass_path() {
        let pass = H2Pass {
            path: "shaders\\shader_passes\\transparent\\lit".to_owned(),
            implementation: 0,
            vertex_shader: String::new(),
            blend: None,
            depth: H2Depth { compare: None, write: false },
            channels: String::new(),
            custom_channels: None,
            alpha_test: None,
            textures: Vec::new(),
            parameters: Vec::new(),
            vertex_constants: Vec::new(),
            animations: Default::default(),
        };
        assert_eq!(pass.pixel_shader_file(), "shaders_shader_passes_transparent_lit_0.psh");
    }

    fn permutation(weight: f32, lower: [f32; 3], upper: [f32; 3]) -> H2ChangePermutation {
        H2ChangePermutation { weight, lower, upper, variant: String::new() }
    }

    #[test]
    fn a_ce_object_at_the_origin_picks_by_channel() {
        let red = [1.0, 0.0, 0.0];
        let blue = [0.0, 0.0, 1.0];
        let list = [permutation(0.3, red, red), permutation(0.7, blue, blue)];
        // Channel 0 hashes to 0, and 1 to 0.129: both inside the first share.
        assert_eq!(ce_change_color(&list, 0), red);
        assert_eq!(ce_change_color(&list, 1), red);
        // Channel 3 hashes to 0.387, into the second.
        assert_eq!(ce_change_color(&list, 3), blue);
        assert_eq!(ce_change_color(&[], 3), [1.0; 3]);
        // No weights shares evenly: 0.387 is the first of two.
        let even = [permutation(0.0, red, red), permutation(0.0, blue, blue)];
        assert_eq!(ce_change_color(&even, 3), red);
    }

    #[test]
    fn ce_colors_blend_the_short_way_round_the_hue() {
        // Red and a red just shy of the wrap meet near red, not across cyan.
        let mixed = hsv_interpolate([1.0, 0.0, 0.0], [1.0, 0.0, 0.2], 0.5, false);
        assert!(mixed[0] > 0.99 && mixed[1] < 0.01, "{mixed:?}");
        // Unlike an RGB blend, yellow keeps its brightness between red and green.
        let middle = hsv_interpolate([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, false);
        assert!((middle[0] - 1.0).abs() < 1e-5 && (middle[1] - 1.0).abs() < 1e-5, "{middle:?}");
        // The long way from red to green goes through blue.
        let long = hsv_interpolate([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, true);
        assert!(long[2] > 0.99 && long[1] < 0.01, "{long:?}");
        // A color survives the trip.
        let color = [0.34, 0.4, 0.15];
        let back = hsv_to_rgb(rgb_to_hsv(color));
        assert!(color.iter().zip(back).all(|(a, b)| (a - b).abs() < 1e-5), "{back:?}");
    }
}
