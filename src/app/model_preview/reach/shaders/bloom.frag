#version 410 core
uniform sampler2D source;
uniform sampler2D added;
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
        color = vec4(texture(source, uv).rgb, 1.0);
    } else if (pass == 1) {
        vec4 s = clamp(texture(source, uv), 0.0, 1.0);
        vec3 c = s.rgb * 8.0;
        float intensity = dot(c, vec3(0.299, 0.587, 0.114));
        vec3 b = c * (scale.x * intensity + scale.y);
        color = min(vec4(b, intensity) * 0.25, vec4(8.0));
    } else if (pass == 2) {
        color = texture(source, uv);
    } else if (pass == 3) {
        vec4 a = vec4(0.0);
        for (int y = -3; y <= 3; y += 2) {
            for (int x = -3; x <= 3; x += 2) {
                a += t(vec2(x, y)) * 8.0;
            }
        }
        a += 1e-8;
        color = min(a * 0.25, vec4(8.0)) * 0.03125;
    } else if (pass == 4) {
        vec4 a = 80.0 * t(vec2(-4.1, -0.5)) + 960.0 * t(vec2(-2.3, -0.5)) + 2016.0 * t(vec2(-0.5, -0.5))
            + 960.0 * t(vec2(1.3, -0.5)) + 80.0 * t(vec2(3.1, -0.5));
        color = min(a * 0.0078125, vec4(8.0)) * 0.03125;
    } else if (pass == 5) {
        vec4 a = 8.0 * (0.01953125 * t(vec2(0.5, 4.1)) + 0.234375 * t(vec2(0.5, 2.3))
            + 0.4921875 * t(vec2(0.5, 0.5)) + 0.234375 * t(vec2(0.5, -1.3))
            + 0.01953125 * t(vec2(0.5, -3.1)));
        color = min(a * scale * 4.0, vec4(8.0)) * 0.03125;
    } else if (pass == 6) {
        vec3 rgb = scale.rgb * 8.0 * texture(source, uv).rgb + 8.0 * texture(added, uv).rgb;
        color = min(vec4(rgb, 1.0) * 4.0, vec4(8.0)) / 32.0;
    } else {
        vec4 b = texture(added, uv);
        vec3 rgb = scale.rgb * 8.0 * texture(source, uv).rgb * (8.0 * b.a) + 8.0 * b.rgb;
        color = min(vec4(rgb * 4.0, b.a * 32.0), vec4(8.0)) * 0.03125;
    }
}
