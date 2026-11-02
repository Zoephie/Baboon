//! The vertex stage for Halo 2's transparent passes.
//!
//! Every transparent pass a model uses names one of five vertex shaders, and
//! they are variations on one program, written out here: a Fresnel tint in
//! `color0`, fog in `color1`, and texture coordinates through per-stage 2D
//! transforms. The preview has no fog, and the pixel shaders scale the scene
//! by `color1.a`, so `color1` is left at 1.

use super::ce_lights::CE_POINT_LIGHT_ROWS;
use super::h2_prt::H2_PRT_REGISTERS;
use super::renderer::model_vertex_prelude;

/// Passes keep values up to register 9 of the vertex shader's bank.
pub(in crate::app) const H2_TEXTURE_ROWS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::app) enum H2VertexStage {
    /// All four texture coordinates through their transforms.
    Default,
    /// `tc0` is the reflection vector. The sky variant only pins the result
    /// to the far plane, which a lone model in the preview does not need.
    Reflection,
    /// The surface pushed along its normal, with 3D coordinates from position.
    Offset,
    /// Tangent space in `tc1..3`, the vector to the eye in their `w`.
    Bumped,
    /// An object's lightmap pass. Every variant has its base coordinates in
    /// `tc0`, the bump map's in `tc2`, the direct light in tangent space in
    /// `tc3`, and the eye and indirect light packed into the two colors. They
    /// differ in the rest.
    Lightmap,
    /// `tc1` is the reflection vector.
    LightmapEnv,
    /// Adds a mask at `tc4`.
    LightmapMark,
    /// Reflection in `tc1`, detail at `tc4`, two masks at `tc5` and `tc6`.
    LightmapEnvStencil,
    /// Masks at `tc4` and `tc5`.
    LightmapMarkStencil,
    /// `tc1` is the tangent-space eye vector, otherwise as `LightmapEnvStencil`.
    LightmapStencil,
    /// Reads the texaccum target back: `tc0` is the pixel's screen position
    /// and `tc1` the reflection vector.
    LightmapTarget,
    /// `LightmapTarget` with the tangent-space eye vector in `tc1`.
    LightmapTargetEye,
    /// `tc1` is the tangent-space eye vector, and only base and bump are read.
    LightmapEye,
    /// `LightmapEye` with a chained map at `tc4` and a third at `tc5`.
    LightmapEyeBlend,
    /// A bump at `tc0`, and in `tc1` and `tc2` the rows that turn it into a
    /// bent screen position to read the frame behind at.
    ActiveCamo,
    /// A surface looking down at the frame behind it, by world position. The
    /// game ripples this by registers no tag holds, so it stands still here.
    Water,
    /// The model as its key light sees it, for depth alone.
    ShadowCaster,
    /// The model in view, with its place on the shadow map in `tc0`.
    ShadowMask,
    /// One map at `tc0`.
    TextureCamera,
    /// The frame behind it at `tc0`, and three maps laid out across the view.
    Hologram,
    /// Precomputed radiance transfer, reading the texaccum target at `tc0`.
    Prt,
    /// `Prt` with its own base at `tc0` and a detail map at `tc4`.
    PrtDetail,
    /// Fills the texaccum target: base, detail and two masks.
    Texaccum,
    /// Self-illumination: four coordinates off one base transform.
    Emissive,
    /// A reflection without a bump basis: the eye in `tc2`, reflection in `tc3`.
    Environment,
    /// A sphere light's diffuse pass: the light in tangent space in `tc1`, and
    /// the position within its falloff volume in `tc3`.
    LightDiffuse,
    /// `LightDiffuse` reading its albedo from the texaccum target.
    LightDiffuseTarget,
    /// `LightDiffuse` with a detail map at `tc4`.
    LightDiffuseDetail,
    /// A frustum light's diffuse pass: the light in tangent space in `tc1`, the
    /// gel's projection in `tc3`, its falloff in one color and near plane in
    /// the other.
    LightGel,
    /// `LightGel` reading its albedo from the texaccum target.
    LightGelTarget,
    /// `LightGel` with a detail map at `tc4`.
    LightGelDetail,
    /// A sphere light's specular pass: light and eye in tangent space.
    LightSpecular,
    /// A frustum light's diffuse and specular together, through the frame's
    /// alpha: light and eye in tangent space, the falloff in a color.
    LightFrustum,
    /// `LightFrustum` reading its albedo from the texaccum target.
    LightFrustumTarget,
    /// `LightFrustum` with a detail map at `tc4`.
    LightFrustumDetail,
    /// Halo CE's `model.vsh` for a lone object: the two distant lights and
    /// ambient of `CeModelRows`, and the object's point lights, with no fog.
    CeModel,
    /// Halo CE's `transparent_generic`: four maps through their transforms,
    /// the fades in the colors.
    CeTransparent,
    /// With the first map a cube, by the reflection.
    CeTransparentReflection,
    /// By the normal.
    CeTransparentObject,
    /// By the vector to the eye.
    CeTransparentViewer,
    /// With the first map laid across the view.
    CeTransparentScreen,
    /// Halo CE's glass: two maps by scale, lit as `CeModel` is.
    CeGlassLit,
    /// A reflection with no bump map to bend it.
    CeGlassFlat,
    /// One map, as it lies.
    CeMeter,
    /// Water's base map, and the eye in tangent space.
    CeWaterOpacity,
    /// Two 3D noise lookups by position, pushed out along the normal.
    CePlasma,
    /// `model_active_camouflage.vsh`: the view-space normal, and how far the
    /// surface bends the frame behind it at this distance.
    CeActiveCamo,
}

/// The first map's cube coordinate, from `cube` and the map's transform, and
/// everything else as `CE_TRANSPARENT` has it.
const CE_TRANSPARENT_CUBE: &str = "vec3 across = u_texture_rows[0].xyz;
    vec3 up = u_texture_rows[1].xyz;
    v_tc0 = vec4(dot(cube, across), dot(cube, up), dot(cube, cross(across, up)), 1.0);
    v_tc1.xy = vec2(row(2, uv), row(3, uv));
    v_tc2.xy = vec2(row(4, uv), row(5, uv));
    v_tc3.xy = vec2(row(6, uv), row(7, uv));
    float face = abs(dot(normal, u_view_back));
    v_color0 = vec4(0.0, 0.0, 0.0, u_texture_rows[8].x);
    v_color1 = vec4(vec3(face), 1.0 - face) * u_texture_rows[8].x;";

/// `transparent_generic.vsh` without its fog, whatever the first map does.
const CE_TRANSPARENT: &str = "v_tc0.xy = vec2(row(0, uv), row(1, uv));
    v_tc1.xy = vec2(row(2, uv), row(3, uv));
    v_tc2.xy = vec2(row(4, uv), row(5, uv));
    v_tc3.xy = vec2(row(6, uv), row(7, uv));
    float face = abs(dot(normal, u_view_back));
    v_color0 = vec4(0.0, 0.0, 0.0, u_texture_rows[8].x);
    v_color1 = vec4(vec3(face), 1.0 - face) * u_texture_rows[8].x;";

impl H2VertexStage {
    /// From a pass's `vertex shader` reference.
    pub(in crate::app) fn from_path(path: &str) -> Option<Self> {
        match path.rsplit(['\\', '/']).next()? {
            "transparent_generic_default" | "transparent_generic_default_sky" => Some(Self::Default),
            "transparent_generic_reflection" | "transparent_generic_reflection_sky" => {
                Some(Self::Reflection)
            }
            "transparent_generic_offset" => Some(Self::Offset),
            "stencil_env_bumped_tangentspace" => Some(Self::Bumped),
            "lightmap_dynamic_tex_detail" => Some(Self::Lightmap),
            "lightmap_dynamic_tex_env" => Some(Self::LightmapEnv),
            "lightmap_dynamic_tex_mark" => Some(Self::LightmapMark),
            "lightmap_dynamic_tex_env_stencil" => Some(Self::LightmapEnvStencil),
            "lightmap_dynamic_tex_mark_stencil" => Some(Self::LightmapMarkStencil),
            "lightmap_dynamic_tex_stencil" => Some(Self::LightmapStencil),
            "lightmap_dynamic_env" => Some(Self::LightmapTarget),
            "lightmap_dynamic" => Some(Self::LightmapTargetEye),
            "hologram" => Some(Self::Hologram),
            "texture_camera" => Some(Self::TextureCamera),
            "water_pool" | "water_pool_static" => Some(Self::Water),
            "transparent_active_camo_viewer_normal" => Some(Self::ActiveCamo),
            "prt" => Some(Self::Prt),
            "prt_lightmap" => Some(Self::PrtDetail),
            "lightmap_dynamic_tex" => Some(Self::LightmapEye),
            "lightmap_dynamic_tex_detail_blend" => Some(Self::LightmapEyeBlend),
            "stencil_texture" => Some(Self::Texaccum),
            "stencil_emissive" => Some(Self::Emissive),
            "stencil_env" => Some(Self::Environment),
            "lightaccum_sphere_diffuse_tex" => Some(Self::LightDiffuse),
            "lightaccum_sphere_diffuse" => Some(Self::LightDiffuseTarget),
            "lightaccum_sphere_diffuse_tex_detail" => Some(Self::LightDiffuseDetail),
            "lightaccum_frustum_no_specular_gel_tex" => Some(Self::LightGel),
            "lightaccum_frustum_no_specular_gel" => Some(Self::LightGelTarget),
            "lightaccum_frustum_no_specular_gel_tex_detail" => Some(Self::LightGelDetail),
            "stencil_light_specular_tangentspace_sphere" => Some(Self::LightSpecular),
            "lightaccum_frustum_tex" => Some(Self::LightFrustum),
            "lightaccum_frustum" => Some(Self::LightFrustumTarget),
            "lightaccum_frustum_tex_detail" => Some(Self::LightFrustumDetail),
            _ => None,
        }
    }

    fn outputs(self) -> &'static str {
        match self {
            Self::Default => {
                "v_tc0.xy = vec2(row(0, uv), row(1, uv));
    v_tc1.xy = vec2(row(2, uv), row(3, uv));
    v_tc2.xy = vec2(row(4, uv), row(5, uv));
    v_tc3.xy = vec2(row(6, uv), row(7, uv));"
            }
            Self::Reflection => {
                "v_tc0.xyz = normal * dot(to_eye, normal) * u_reflection_scale - to_eye;
    v_tc1.xy = vec2(row(2, uv), row(3, uv));
    v_tc2.xy = vec2(row(4, uv), row(5, uv));
    v_tc3.xy = vec2(row(6, uv), row(7, uv));"
            }
            Self::Offset => {
                "v_tc0.xyz = vec3(row(0, position), row(1, position), row(2, position));
    v_tc1.xyz = vec3(row(3, position), row(4, position), row(5, position));"
            }
            Self::Lightmap
            | Self::LightmapEnv
            | Self::LightmapMark
            | Self::LightmapEnvStencil
            | Self::LightmapMarkStencil
            | Self::LightmapStencil
            | Self::LightmapTarget
            | Self::LightmapTargetEye
            | Self::LightmapEye
            | Self::LightmapEyeBlend
            | Self::Texaccum => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc0.xy = base;
    v_tc2.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc3.xyz = tangent_space(-u_light_direct, tangent, binormal, normal);
    vec3 eye_vector = tangent_space(to_eye, tangent, binormal, normal);
    v_color0 = vec4(normalize(eye_vector) * 0.5 + 0.5, 1.0);
    vec3 indirect = tangent_space(-u_light_indirect, tangent, binormal, normal);
    v_color1 = vec4(normalize(indirect) * 0.5 + 0.5, 1.0);"
            }
            Self::ActiveCamo => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc0.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_color0 = vec4(tangent_space(u_view_back, tangent, binormal, normal) * 0.5 + 0.5, 1.0);
    vec3 view_normal = vec3(dot(u_view_right, normal), dot(u_view_up, normal), dot(u_view_back, normal));
    v_tc3.xyz = view_normal;
    float bend = log2(max(view_normal.z, 0.0));
    vec4 refraction = vec4(
        view_normal.xy * pow(2.0, bend * u_texture_rows[9].y) * u_texture_rows[8].y + u_texture_rows[8].zw,
        vec2(pow(2.0, bend * u_texture_rows[9].x) * u_texture_rows[8].x));
    refraction *= u_texture_rows[9].z / 320.0;
    vec2 screen = gl_Position.xy / gl_Position.w * 0.5 + 0.5;
    vec3 across = u_view_right * refraction.z;
    vec3 down = u_view_up * refraction.w;
    v_tc1 = vec4(dot(across, tangent), dot(across, binormal), screen.x + refraction.x, 1.0);
    // The game's screen runs down, a framebuffer's up.
    v_tc2 = vec4(dot(down, tangent), dot(down, binormal), screen.y - refraction.y, 1.0);
    v_tc2.xy = -v_tc2.xy;
    v_fog = dot(-to_eye, u_view_back) * u_texture_rows[5].x + u_texture_rows[5].y;"
            }
            Self::Water => {
                "v_tc0.xy = position.xy * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc3.xy = vec2(0.5, normalize(to_eye).z * 0.5 + 0.5);"
            }
            Self::ShadowCaster => "gl_Position = u_shadow_matrix * vec4(position, 1.0);",
            Self::ShadowMask => "v_tc0 = u_shadow_matrix * vec4(position, 1.0);",
            Self::TextureCamera => {
                "v_tc0.xy = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;"
            }
            Self::Hologram => {
                "vec3 offset = position - u_eye;
    vec2 across = vec2(dot(offset, u_view_right), dot(offset, u_view_up));
    v_tc1.xy = across * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc2.xy = across * u_texture_rows[1].xy + u_texture_rows[1].zw;
    v_tc3.xy = across * u_texture_rows[2].xy + u_texture_rows[2].zw;"
            }
            Self::Prt | Self::PrtDetail => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc1.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    vec3 eye_unit = normalize(to_eye);
    float eye_facing = dot(normal, eye_unit);
    v_tc2.xyz = normal * eye_facing * u_reflection_scale - eye_unit;
    int cluster = int(a_prt_cluster + 0.5) * 4;
    vec3 transfer = u_prt[cluster].xyz + vec3(
        dot(a_prt_weights, u_prt[cluster + 1]),
        dot(a_prt_weights, u_prt[cluster + 2]),
        dot(a_prt_weights, u_prt[cluster + 3]));
    v_color0 = vec4(transfer * u_texture_rows[3].x, dot(transfer, transfer));
    v_tc3.xyz = tangent_space(u_prt_light, tangent, binormal, normal);
    vec3 halfway = normalize(u_prt_light + eye_unit);
    v_color1 = vec4(tangent_space(halfway, tangent, binormal, normal) * 0.5 + 0.5, eye_facing * eye_facing);"
            }
            Self::Emissive => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc0.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;
    v_tc1.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc2.xy = base * u_texture_rows[3].xy + u_texture_rows[3].zw;
    v_tc3.xy = base * u_texture_rows[4].xy + u_texture_rows[4].zw;"
            }
            Self::Environment => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc0.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;
    v_tc2.xyz = tangent_space(to_eye, tangent, binormal, normal);
    v_tc3.xyz = normal * dot(normal, to_eye) * u_reflection_scale - to_eye;"
            }
            Self::LightDiffuse | Self::LightDiffuseTarget | Self::LightDiffuseDetail => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    vec3 to_light = u_light_position - position;
    v_tc0.xy = base;
    v_tc1.xyz = tangent_space(to_light, tangent, binormal, normal);
    v_tc2.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc3.xyz = to_light * -u_light_falloff + 0.5;"
            }
            Self::LightGel | Self::LightGelTarget | Self::LightGelDetail => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    vec3 to_light = u_light_position - position;
    v_tc0.xy = base;
    v_tc1.xyz = tangent_space(to_light, tangent, binormal, normal);
    v_tc2.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc3 = vec4(dot(to_light, u_gel_rows[0].xyz) + u_gel_rows[0].w,
        dot(to_light, u_gel_rows[1].xyz) + u_gel_rows[1].w,
        0.0, dot(to_light, u_gel_rows[2].xyz));
    v_tc3.z = v_tc3.w;
    v_color0 = vec4(vec3(clamp(dot(to_light, u_gel_rows[2].xyz) + u_gel_rows[2].w, 0.0, 1.0)), 1.0);
    v_color1 = vec4(vec3(clamp(dot(to_light, u_gel_rows[3].xyz) + u_gel_rows[3].w, 0.0, 1.0)), 1.0);"
            }
            Self::LightFrustum | Self::LightFrustumTarget | Self::LightFrustumDetail => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    vec3 to_light = u_light_position - position;
    v_tc0.xy = base;
    v_tc1.xyz = tangent_space(to_light, tangent, binormal, normal);
    v_tc2.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc3.xyz = tangent_space(to_eye, tangent, binormal, normal);
    v_color0 = vec4(vec3(dot(to_light, u_gel_rows[2].xyz) + u_gel_rows[2].w), 1.0);"
            }
            Self::LightSpecular => {
                "vec2 base = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc1.xyz = tangent_space(u_light_position - position, tangent, binormal, normal);
    v_tc2.xy = base * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc3.xyz = tangent_space(to_eye, tangent, binormal, normal);
    v_color0 = u_tint_facing;
    v_color1 = u_tint_glancing;"
            }
            Self::CeModel => {
                "v_tc0.xy = vec2(row(0, uv), row(1, uv));
    v_tc1.xy = v_tc0.xy * u_texture_rows[2].xy;
    v_tc2.xy = v_tc0.xy;
    vec3 surface = normal * u_texture_rows[3].y;
    v_tc3.xyz = surface * dot(to_eye, surface) * 2.0 - to_eye;
    float first = dot(surface, -u_light_direct);
    first = max(first, -first * u_texture_rows[3].x);
    float second = dot(surface, -u_texture_rows[6].xyz);
    v_color0 = vec4(clamp(u_texture_rows[4].rgb + ce_point_light(position, surface)
        + max(first, 0.0) * u_texture_rows[5].rgb
        + max(second, 0.0) * u_texture_rows[7].rgb, 0.0, 1.0), 0.0);
    float view = dot(normalize(to_eye), surface);
    v_color1 = clamp(u_tint_facing * view + u_tint_glancing, 0.0, 1.0);
    if (u_texture_rows[8].x != 0.0) {
        float along = dot(u_view_back, position - u_eye) * -1.0;
        v_color1.a *= clamp((along - u_texture_rows[8].x)
            / (u_texture_rows[8].y - u_texture_rows[8].x), 0.0, 1.0);
    }"
            }
            Self::CeGlassLit => {
                "v_tc0.xy = uv.xy * u_texture_rows[2].xy;
    v_tc1.xy = uv.xy * u_texture_rows[2].zw;
    float first = max(dot(normal, -u_light_direct), 0.0);
    float second = max(dot(normal, -u_texture_rows[6].xyz), 0.0);
    v_color0 = vec4(clamp(u_texture_rows[4].rgb + ce_point_light(position, normal)
        + first * u_texture_rows[5].rgb + second * u_texture_rows[7].rgb, 0.0, 1.0), 1.0);"
            }
            Self::CeGlassFlat => {
                "v_tc0.xy = uv.xy * u_texture_rows[2].xy;
    v_tc1.xyz = tangent_space(u_view_back, tangent, binormal, normal);
    v_tc2.xyz = normal;
    v_tc3.xyz = normal * dot(to_eye, normal) * 2.0 - to_eye;"
            }
            Self::CeMeter => "v_tc0.xy = uv.xy;",
            Self::CeWaterOpacity => {
                "v_tc0.xy = uv.xy;
    v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);"
            }
            Self::CeActiveCamo => {
                "float depth = dot(to_eye, u_view_back);
    float near = clamp(u_texture_rows[0].y / (depth * depth), 0.0, 1.0);
    v_color0 = vec4(u_texture_rows[1].rgb, near);
    v_tc0.xyz = vec3(dot(normal, u_view_right), dot(normal, u_view_up), -dot(normal, u_view_back));
    v_tc1.xyz = vec3(near * u_texture_rows[0].x, 0.0, 1.0 / u_view_size.x);
    // The game's screen runs down, a framebuffer's up.
    v_tc2.xyz = vec3(0.0, -near * u_texture_rows[0].x, 1.0 / u_view_size.y);"
            }
            Self::CePlasma => {
                "vec4 place = vec4(a_position, 1.0);
    v_tc0.xyz = vec3(dot(place, u_texture_rows[0]), dot(place, u_texture_rows[1]),
        dot(place, u_texture_rows[2]));
    v_tc1.xyz = vec3(dot(place, u_texture_rows[3]), dot(place, u_texture_rows[4]),
        dot(place, u_texture_rows[5]));
    v_color0 = clamp(u_texture_rows[7] * dot(normalize(to_eye), normal) + u_texture_rows[6], 0.0, 1.0);"
            }
            Self::CeTransparent => CE_TRANSPARENT,
            Self::CeTransparentReflection => "vec3 cube = normal * dot(to_eye, normal) * 2.0 - to_eye;",
            Self::CeTransparentObject => "vec3 cube = normal;",
            Self::CeTransparentViewer => "vec3 cube = to_eye;",
            Self::CeTransparentScreen => CE_TRANSPARENT,
            Self::Bumped => {
                "v_tc0.xy = uv.xy * u_texture_rows[0].xy + u_texture_rows[0].zw;
    v_tc0.xy = v_tc0.xy * u_texture_rows[2].xy + u_texture_rows[2].zw;
    v_tc1 = vec4(tangent.x, binormal.x, normal.x, to_eye.x);
    v_tc2 = vec4(tangent.y, binormal.y, normal.y, to_eye.y);
    v_tc3 = vec4(tangent.z, binormal.z, normal.z, to_eye.z);
    v_color0 = u_tint_facing;
    v_color1 = u_tint_glancing;"
            }
        }
    }

    /// Whether this is one of the lightmap family, whose texture registers
    /// hold a 2D scale and offset rather than affine rows.
    pub(in crate::app) fn is_lightmap(self) -> bool {
        matches!(
            self,
            Self::Lightmap
                | Self::LightmapEnv
                | Self::LightmapMark
                | Self::LightmapEnvStencil
                | Self::LightmapMarkStencil
                | Self::LightmapStencil
                | Self::LightmapTarget
                | Self::LightmapTargetEye
                | Self::LightmapEye
                | Self::LightmapEyeBlend
                | Self::Texaccum
        )
    }

    /// Whether texture registers hold a 2D scale and offset, not affine rows.
    pub(in crate::app) fn uses_scale_rows(self) -> bool {
        self.is_lightmap()
            || matches!(
                self,
                Self::Bumped
                    | Self::Emissive
                    | Self::Environment
                    | Self::LightDiffuse
                    | Self::LightDiffuseTarget
                    | Self::LightDiffuseDetail
                    | Self::LightGel
                    | Self::LightGelTarget
                    | Self::LightGelDetail
                    | Self::LightSpecular
                    | Self::LightFrustum
                    | Self::LightFrustumTarget
                    | Self::LightFrustumDetail
                    | Self::ActiveCamo
                    | Self::TextureCamera
                    | Self::Water
                    | Self::Hologram
                    | Self::Prt
                    | Self::PrtDetail
            )
    }

    /// Whether registers also carry lone values rather than only map scales.
    pub(in crate::app) fn takes_row_values(self) -> bool {
        matches!(self, Self::ActiveCamo | Self::Offset | Self::Prt | Self::PrtDetail)
    }

    /// What differs between the lightmap variants.
    fn lightmap_outputs(self) -> &'static str {
        const CHAINED: &str = "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;";
        match self {
            Self::LightmapEnv => {
                "v_tc1.xyz = normal * dot(normal, to_eye) * u_reflection_scale - to_eye;"
            }
            Self::LightmapMark => concat!(
                "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc4.xy = base * u_texture_rows[3].xy;"
            ),
            Self::LightmapEnvStencil => concat!(
                "v_tc1.xyz = normal * dot(normal, to_eye) * u_reflection_scale - to_eye;\n    ",
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].xy;\n    ",
                "v_tc6.xy = base * u_texture_rows[3].zw;"
            ),
            Self::LightmapMarkStencil => concat!(
                "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc4.xy = base * u_texture_rows[3].xy;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].zw;"
            ),
            Self::LightmapTarget => concat!(
                "v_tc0 = vec4((gl_Position.xy + gl_Position.w) * 0.5, gl_Position.ww);\n    ",
                "v_tc1.xyz = normal * dot(normal, to_eye) * u_reflection_scale - to_eye;"
            ),
            Self::LightmapTargetEye => concat!(
                "v_tc0 = vec4((gl_Position.xy + gl_Position.w) * 0.5, gl_Position.ww);\n    ",
                "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);"
            ),
            Self::LightmapEye => "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);",
            Self::LightmapEyeBlend => concat!(
                "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);\n    ",
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].xy;"
            ),
            Self::Texaccum => concat!(
                "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc2.xy = base * u_texture_rows[2].xy;\n    ",
                "v_tc3 = vec4(base * u_texture_rows[2].zw, 0.0, 1.0);"
            ),
            Self::LightmapStencil => concat!(
                "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);\n    ",
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].xy;\n    ",
                "v_tc6.xy = base * u_texture_rows[3].zw;"
            ),
            _ => CHAINED,
        }
    }

    /// Only the offset variant moves the drawn surface.
    fn drawn_position(self) -> &'static str {
        match self {
            Self::Offset => "position + normal * u_texture_rows[8].w",
            Self::CePlasma => "position + normal * u_texture_rows[0].z",
            _ => "position",
        }
    }

    pub(in crate::app) fn source(self, version_declaration: &str) -> String {
        let prelude = model_vertex_prelude(version_declaration);
        let (outputs, drawn) = (self.outputs(), self.drawn_position());
        let variant = match self {
            Self::LightDiffuseTarget | Self::LightGelTarget | Self::LightFrustumTarget => {
                "v_tc0 = vec4((gl_Position.xy + gl_Position.w) * 0.5, gl_Position.ww);"
            }
            Self::CeTransparentReflection | Self::CeTransparentObject | Self::CeTransparentViewer => {
                CE_TRANSPARENT_CUBE
            }
            Self::CeTransparentScreen => {
                "v_tc0 = vec4(row(0, vec3(gl_Position.xy, 0.0)) + u_texture_rows[0].w * (gl_Position.w - 1.0), \
                 row(1, vec3(gl_Position.xy, 0.0)) + u_texture_rows[1].w * (gl_Position.w - 1.0), 0.0, gl_Position.w);"
            }
            Self::LightDiffuseDetail | Self::LightGelDetail | Self::LightFrustumDetail => {
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;"
            }
            Self::Hologram | Self::Prt => {
                "v_tc0 = vec4((gl_Position.xy + gl_Position.w) * 0.5, gl_Position.ww);"
            }
            Self::Water => concat!(
                "v_tc1 = vec4((gl_Position.xy / gl_Position.w + 1.0) * 0.5, 0.0, 1.0);\n    ",
                "v_tc2 = vec4((gl_Position.xy + gl_Position.w) * 0.5, gl_Position.ww);"
            ),
            Self::PrtDetail => concat!(
                "v_tc0.xy = base;\n    ",
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;"
            ),
            _ if self.is_lightmap() => self.lightmap_outputs(),
            _ => "",
        };
        let prt = if matches!(self, Self::Prt | Self::PrtDetail) {
            format!(
                "in float a_prt_cluster;\nin vec4 a_prt_weights;\n\
                 uniform vec4 u_prt[{H2_PRT_REGISTERS}];\nuniform vec3 u_prt_light;\n"
            )
        } else {
            String::new()
        };
        format!(
            "{prelude}{prt}\
invariant gl_Position;
out vec4 v_color0;
out vec4 v_color1;
out vec4 v_tc0;
out vec4 v_tc1;
out vec4 v_tc2;
out vec4 v_tc3;
out vec4 v_tc4;
out vec4 v_tc5;
out vec4 v_tc6;
out vec4 v_tc7;
out float v_fog;
uniform vec3 u_eye;
uniform vec2 u_view_size;
uniform vec3 u_view_right;
uniform vec3 u_view_up;
uniform vec3 u_view_back;
uniform mat4 u_shadow_matrix;
uniform vec4 u_texture_rows[{H2_TEXTURE_ROWS}];
uniform vec4 u_tint_facing;
uniform vec4 u_tint_glancing;
uniform float u_reflection_scale;
uniform vec3 u_light_direct;
uniform vec3 u_light_indirect;
uniform vec3 u_light_position;
uniform float u_light_falloff;
uniform vec4 u_gel_rows[4];
uniform vec4 u_ce_point[{CE_POINT_LIGHT_ROWS}];
vec3 tangent_space(vec3 value, vec3 tangent, vec3 binormal, vec3 normal) {{
    return vec3(dot(value, tangent), dot(value, binormal), dot(value, normal));
}}
float row(int index, vec3 point) {{
    return dot(u_texture_rows[index].xyz, point) + u_texture_rows[index].w;
}}
vec3 ce_point_light(vec3 position, vec3 normal) {{
    vec3 sum = vec3(0.0);
    for (int light = 0; light < 2; light++) {{
        vec4 place = u_ce_point[light * 3];
        vec4 aim = u_ce_point[light * 3 + 1];
        vec4 color = u_ce_point[light * 3 + 2];
        vec3 to_light = place.xyz - position;
        float squared = dot(to_light, to_light);
        to_light /= sqrt(max(squared, 0.000001));
        float near = max(1.0 - squared * place.w, 0.0);
        float facing = clamp(dot(to_light, normal), 0.0, 1.0);
        float spot = max(dot(to_light, -aim.xyz) * aim.w + color.w, 0.0);
        // `model.vsh` caps only the second light's spot at 1.
        sum += near * facing * (light == 1 ? min(spot, 1.0) : spot) * color.rgb;
    }}
    return sum;
}}
void main() {{
    vec3 position;
    vec3 normal_in;
    vec3 tangent;
    vec3 binormal;
    skinned_pose(position, normal_in, tangent, binormal);
    vec3 normal = length(normal_in) > 0.0001 ? normalize(normal_in) : vec3(0.0, 0.0, 1.0);
    vec3 uv = vec3(a_texcoord, 0.0);
    vec3 to_eye = u_eye - position;
    float facing = max(dot(normal, normalize(to_eye)), 0.0);
    gl_Position = clip_position({drawn});
    v_color0 = u_tint_facing * facing + u_tint_glancing * (1.0 - facing);
    v_color1 = vec4(1.0);
    v_tc0 = vec4(0.0, 0.0, 0.0, 1.0);
    v_tc1 = v_tc0;
    v_tc2 = v_tc0;
    v_tc3 = v_tc0;
    v_tc4 = v_tc0;
    v_tc5 = v_tc0;
    v_tc6 = v_tc0;
    v_tc7 = vec4(0.0, 0.0, 0.0, 1.0);
    v_fog = 1.0;
    {outputs}
    {variant}
}}
"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::h2_pixel_shader::{PixelShaderOptions, translate};

    const STAGES: [H2VertexStage; 47] = [
        H2VertexStage::CeActiveCamo,
        H2VertexStage::CePlasma,
        H2VertexStage::CeGlassLit,
        H2VertexStage::CeGlassFlat,
        H2VertexStage::CeMeter,
        H2VertexStage::CeWaterOpacity,
        H2VertexStage::CeModel,
        H2VertexStage::CeTransparent,
        H2VertexStage::CeTransparentReflection,
        H2VertexStage::CeTransparentObject,
        H2VertexStage::CeTransparentViewer,
        H2VertexStage::CeTransparentScreen,
        H2VertexStage::LightDiffuseTarget,
        H2VertexStage::LightDiffuseDetail,
        H2VertexStage::LightDiffuse,
        H2VertexStage::LightGel,
        H2VertexStage::LightGelTarget,
        H2VertexStage::LightGelDetail,
        H2VertexStage::LightSpecular,
        H2VertexStage::LightFrustum,
        H2VertexStage::LightFrustumTarget,
        H2VertexStage::LightFrustumDetail,
        H2VertexStage::Emissive,
        H2VertexStage::Environment,
        H2VertexStage::LightmapTarget,
        H2VertexStage::LightmapTargetEye,
        H2VertexStage::ActiveCamo,
        H2VertexStage::ShadowCaster,
        H2VertexStage::ShadowMask,
        H2VertexStage::TextureCamera,
        H2VertexStage::Water,
        H2VertexStage::Hologram,
        H2VertexStage::Prt,
        H2VertexStage::PrtDetail,
        H2VertexStage::LightmapEye,
        H2VertexStage::LightmapEyeBlend,
        H2VertexStage::Texaccum,
        H2VertexStage::Lightmap,
        H2VertexStage::LightmapEnv,
        H2VertexStage::LightmapMark,
        H2VertexStage::LightmapEnvStencil,
        H2VertexStage::LightmapMarkStencil,
        H2VertexStage::LightmapStencil,
        H2VertexStage::Default,
        H2VertexStage::Reflection,
        H2VertexStage::Offset,
        H2VertexStage::Bumped,
    ];

    #[test]
    fn every_transparent_vertex_shader_has_a_stage() {
        let dir = "rasterizer\\vertex_shaders_dx9\\";
        for (name, stage) in [
            ("transparent_generic_default", H2VertexStage::Default),
            ("transparent_generic_reflection", H2VertexStage::Reflection),
            ("transparent_generic_reflection_sky", H2VertexStage::Reflection),
            ("transparent_generic_offset", H2VertexStage::Offset),
            ("stencil_env_bumped_tangentspace", H2VertexStage::Bumped),
        ] {
            assert_eq!(H2VertexStage::from_path(&format!("{dir}{name}")), Some(stage));
        }
        assert_eq!(H2VertexStage::from_path("rasterizer\\unknown"), None);
    }

    /// Stages must link with what the translator produces, so a varying
    /// renamed on one side fails here rather than as a preview drawing nothing.
    #[test]
    fn every_stage_links_with_a_translated_pixel_shader() {
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH");
            return;
        }
        let source = "float4 c1 : register(C1);\nfloat4 main(\n)\n{\n    float4 r0;\n    \
                      r0= color0 * color1 + tc0 + tc1 + tc2 + tc3 + tc7 + fog;\n    \
                      return r0 * c1;\n}\n";
        let fragment =
            translate(source, "#version 330\n", &PixelShaderOptions::default()).unwrap().glsl;
        let dir = crate::core::test_kits::unique_temp_dir("h2_vertex_stages");
        let pixel = dir.join("pass.frag");
        std::fs::write(&pixel, fragment).unwrap();
        for stage in STAGES {
            let vertex = dir.join("pass.vert");
            std::fs::write(&vertex, stage.source("#version 330\n")).unwrap();
            let output = std::process::Command::new("glslangValidator")
                .arg("-l")
                .arg(&vertex)
                .arg(&pixel)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{stage:?} does not link:\n{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    /// With `BLAM_TEST_H2EK`: the texaccum shader links with its stage.
    #[test]
    fn the_texaccum_stage_links_with_the_kit_shader() {
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH");
            return;
        }
        let file = std::path::Path::new(&root)
            .join("rasterizer/pixel_shaders_dx9")
            .join("shaders_shader_passes_texture_mapping_default_texture_zwrite_0.psh");
        let source = String::from_utf8_lossy(&std::fs::read(file).unwrap()).into_owned();
        let dir = crate::core::test_kits::unique_temp_dir("h2_texaccum_stage");
        let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
        std::fs::write(&vertex, H2VertexStage::Texaccum.source("#version 330\n")).unwrap();
        let shader = translate(&source, "#version 330\n", &PixelShaderOptions::default()).unwrap();
        std::fs::write(&pixel, shader.glsl).unwrap();
        let output = std::process::Command::new("glslangValidator")
            .arg("-l")
            .arg(&vertex)
            .arg(&pixel)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    }

    /// With `BLAM_TEST_H2EK`: real object lightmap shaders link with their
    /// stages, in every specular variant.
    #[test]
    fn lightmap_stages_link_with_the_kit_shaders() {
        let Ok(root) = std::env::var("BLAM_TEST_H2EK") else {
            eprintln!("skipping: BLAM_TEST_H2EK is not set");
            return;
        };
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH");
            return;
        }
        let directory = std::path::Path::new(&root).join("rasterizer/pixel_shaders_dx9");
        let dir = crate::core::test_kits::unique_temp_dir("h2_lightmap_stages");
        for (pass, stage) in [
            ("lightmap_tex_detail", H2VertexStage::Lightmap),
            ("lightmap_tex_env", H2VertexStage::LightmapEnv),
            ("lightmap_tex_four_change_color", H2VertexStage::LightmapMark),
            ("lightmap_tex_env_detail", H2VertexStage::LightmapEnvStencil),
            ("lightmap_tex_detail_blend_specular", H2VertexStage::LightmapMarkStencil),
            ("lightmap_honor_guard", H2VertexStage::LightmapStencil),
            ("lightmap_env", H2VertexStage::LightmapTarget),
        ] {
            let file = directory.join(format!("shaders_shader_passes_lightmap_{pass}_3.psh"));
            let source = String::from_utf8_lossy(&std::fs::read(file).unwrap()).into_owned();
            let vertex = dir.join("pass.vert");
            std::fs::write(&vertex, stage.source("#version 330\n")).unwrap();
            for specular_type in 0..4 {
                let options = PixelShaderOptions {
                    cases: std::collections::HashMap::from([(
                        "specular_type".to_owned(),
                        specular_type,
                    )]),
                    alpha_test: None,
                };
                let Ok(shader) = translate(&source, "#version 330\n", &options) else {
                    continue;
                };
                let pixel = dir.join("pass.frag");
                std::fs::write(&pixel, shader.glsl).unwrap();
                let output = std::process::Command::new("glslangValidator")
                    .arg("-l")
                    .arg(&vertex)
                    .arg(&pixel)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{pass} specular {specular_type} does not link:\n{}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
        }
    }
}
