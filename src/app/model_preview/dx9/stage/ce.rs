//! Halo CE's stages' GLSL: what each writes to its outputs, and the
//! pieces its transparent stages draw with.

use super::Dx9VertexStage;

/// Point lights a model takes: position and inverse radius squared, direction
/// and spot A, color and spot B, for each of two.
pub(crate) const CE_POINT_LIGHT_ROWS: usize = 6;

/// The first map's cube coordinate, from `cube` and the map's transform, and
/// everything else as `CE_TRANSPARENT` has it.
pub(super) const CE_TRANSPARENT_CUBE: &str = include_str!("shaders/ce_transparent_cube.glsl");

/// `transparent_generic.vsh` without its fog, whatever the first map does.
pub(super) const CE_TRANSPARENT: &str = include_str!("shaders/ce_transparent.glsl");

impl Dx9VertexStage {
    /// What Halo CE's stages write in [`Dx9VertexStage::outputs`]'s place:
    /// the rest of the stages are Halo 2's and the shared ones.
    pub(super) fn ce_outputs(self) -> &'static str {
        match self {
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
            _ => "",
        }
    }
}
