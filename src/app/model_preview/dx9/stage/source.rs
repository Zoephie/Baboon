//! A stage's GLSL: its outputs, its drawn position and its full source.

use super::*;
use super::super::super::gl::glsl::fill;

impl Dx9VertexStage {
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
            Self::CeModel
            | Self::CeGlassLit
            | Self::CeGlassFlat
            | Self::CeMeter
            | Self::CeWaterOpacity
            | Self::CeActiveCamo
            | Self::CePlasma
            | Self::CeTransparent
            | Self::CeTransparentReflection
            | Self::CeTransparentObject
            | Self::CeTransparentViewer
            | Self::CeTransparentScreen => self.ce_outputs(),
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

    /// The offset and Halo CE plasma variants move the drawn surface out along
    /// the normal; the rest draw it where it is.
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
            fill(include_str!("shaders/prt.glsl"), &[("prt_registers", &H2_PRT_REGISTERS)])
        } else {
            String::new()
        };
        fill(
            include_str!("shaders/stage.vert"),
            &[
                ("prelude", &prelude),
                ("prt", &prt),
                ("texture_rows", &H2_TEXTURE_ROWS),
                ("ce_point_rows", &CE_POINT_LIGHT_ROWS),
                ("drawn", &drawn),
                ("outputs", &outputs),
                ("variant", &variant),
            ],
        )
    }
}
