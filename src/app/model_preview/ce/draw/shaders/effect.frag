#version 330
in vec2 v_uv;
in vec2 v_unit;
in vec4 v_clip;
// The vertex's color, as the game's effect vertices carry it.
in vec4 v_color;
uniform sampler2D u_map;
uniform sampler2D u_second;
uniform int u_blend;
uniform int u_nonlinear;
// 0 none, 1 a second color map over the sprite, 2 over the screen, 3 zsprite.
uniform int u_mode;
uniform vec4 u_rows[2];
// The zsprite's radius, and how clip z and w move per unit toward the eye.
uniform vec3 u_zsprite;
out vec4 out_color;
void main() {
    vec4 t0 = texture(u_map, v_uv);
    vec4 r0 = vec4(u_nonlinear != 0 ? mix(pow(t0.rgb, vec3(4.0)), t0.rgb, v_color.rgb) : t0.rgb * v_color.rgb, t0.a);
    if (u_mode == 1 || u_mode == 2) {
        vec4 at = u_mode == 1 ? vec4(v_uv, 0.0, 1.0) : vec4(v_clip.xy / v_clip.w, 1.0, 1.0);
        vec4 t1 = texture(u_second, vec2(dot(at, u_rows[0]), dot(at, u_rows[1])));
        r0 *= t1;
    }
    float depth = gl_FragCoord.z;
    if (u_mode == 3) {
        // The map's green says how far toward the eye the pixel stands, never
        // through the near plane.
        float toward = u_zsprite.x * texture(u_second, v_unit).g;
        if (u_zsprite.z > 0.0) {
            toward = min(toward, max((v_clip.w - 0.001) / u_zsprite.z, 0.0));
        }
        float z = v_clip.z - toward * u_zsprite.y;
        float w = v_clip.w - toward * u_zsprite.z;
        depth = (z / w) * 0.5 + 0.5;
    }
    gl_FragDepth = depth;
    float a = v_color.a;
    // `sub_85F3B0` draws min with the multiply shader, and subtract and max
    // with the add one.
    if (u_blend == 0) {
        r0.a *= a;
    } else if (u_blend == 1 || u_blend == 5) {
        r0.rgb = mix(vec3(1.0), r0.rgb, a);
    } else if (u_blend == 2) {
        r0.rgb = mix(vec3(0.5), r0.rgb, a);
    } else if (u_blend == 3 || u_blend == 4 || u_blend == 6) {
        r0.rgb *= a;
    } else if (u_blend == 7) {
        r0 *= a;
    }
    out_color = r0;
}
