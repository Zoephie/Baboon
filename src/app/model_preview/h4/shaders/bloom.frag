#version 410 core
uniform sampler2D source;
uniform sampler2D added;
uniform sampler3D grading;
uniform bool graded;
uniform bool keeps_alpha;
uniform vec3 grading_scale;
uniform vec3 grading_offset;
uniform int pass;
uniform vec2 target_size;
uniform vec2 source_texel;
uniform vec4 scale;
out vec4 color;
vec2 uv;
vec4 t(vec2 offset) {
    return texture(source, uv + offset * source_texel);
}
void main() {
    uv = gl_FragCoord.xy / target_size;
    if (pass == 0) {
        color = texture(source, uv);
    } else if (pass == 1) {
        vec4 s = texture(source, uv);
        s.rgb = clamp(s.rgb, 0.0, 1.0);
        s.a = min(s.a, 1.0 / 32.0);
        // DARK_COLOR_MULTIPLIER, `ps_exposure.g`.
        s.rgb *= 8.0;
        float intensity = graded
            ? textureLod(grading, s.rgb * grading_scale + grading_offset, 0.0).a
            : dot(s.rgb, vec3(0.3086, 0.6094, 0.0820));
        float bloom = scale.x * intensity + scale.z * s.a + scale.y;
        color = vec4(s.rgb * bloom, keeps_alpha ? s.a : intensity);
    } else if (pass == 2) {
        vec4 a = vec4(0.00000001);
        for (int y = -3; y <= 3; y += 2) {
            for (int x = -3; x <= 3; x += 2) {
                a += t(vec2(x, -y));
            }
        }
        color = a * (1.0 / 16.0);
    } else if (pass == 3) {
        // `tex2DOffsetLinear(.., -0.5, 0.5)` on each tap.
        color = 10.0 / 512.0 * t(vec2(-4.1 - 0.5, -0.5)) + 10.0 / 512.0 * t(vec2(3.1 - 0.5, -0.5))
            + 120.0 / 512.0 * t(vec2(1.3 - 0.5, -0.5)) + 120.0 / 512.0 * t(vec2(-2.3 - 0.5, -0.5))
            + 252.0 / 512.0 * t(vec2(-0.5, -0.5));
    } else if (pass == 4) {
        // `ps_kernel` (X360 0x822658A8): x 0.5, y -4.1 .. 3.1.
        vec4 a = 0.01953125 * t(vec2(0.5, 4.1)) + 0.234375 * t(vec2(0.5, 2.3)) + 0.4921875 * t(vec2(0.5, 0.5))
            + 0.234375 * t(vec2(0.5, -1.3)) + 0.01953125 * t(vec2(0.5, -3.1));
        color = a * scale;
    } else if (pass == 5) {
        vec4 original = texture(source, uv);
        vec4 add = texture(added, uv);
        color = vec4(scale.rgb, dot(scale.rgb, vec3(0.299, 0.587, 0.114))) * original + add;
    } else {
        vec4 source_color = texture(source, uv);
        vec4 add = texture(added, uv);
        color = vec4(scale.rgb * source_color.rgb * add.a + add.rgb, add.a);
    }
}
