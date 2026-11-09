#version 410 core
uniform sampler2D picture;
uniform sampler2D depth;
uniform mat4 screen_to_world;
uniform mat4 world_to_screen;
uniform vec3 moved;
uniform float most_pixels;
uniform float blur_step;
uniform vec2 center_falloff;
out vec4 color;
void main() {
    vec2 size = vec2(textureSize(picture, 0));
    ivec2 at = ivec2(gl_FragCoord.xy);
    vec3 center = texelFetch(picture, at, 0).rgb;
    vec2 from_center = (gl_FragCoord.xy - size * 0.5) * sqrt(center_falloff) / size;
    float weight = clamp(clamp(dot(from_center, from_center), 0.0, 1.0) - 0.1, 0.0, 1.0);
    color = vec4(center, 1.0);
    if (weight > 0.0) {
        float raw = texelFetch(depth, at, 0).r;
        vec4 place = screen_to_world * vec4(gl_FragCoord.xy / size * 2.0 - 1.0, raw * 2.0 - 1.0, 1.0);
        vec4 before = world_to_screen * vec4(place.xyz / place.w + moved, 1.0);
        vec2 delta = gl_FragCoord.xy - (before.xy / before.w * 0.5 + 0.5) * size;
        // Still and with no blur allowed is 0 / 0, which would poison the steps.
        float scale = clamp(most_pixels / max(length(delta), 1e-6), 0.0, 1.0);
        vec2 step_by = delta * blur_step * scale * weight;
        vec3 sum = center;
        for (int sample_at = 1; sample_at <= 6; sample_at++) {
            ivec2 there = clamp(ivec2(gl_FragCoord.xy + step_by * float(sample_at)), ivec2(0), ivec2(size) - 1);
            sum += texelFetch(picture, there, 0).rgb;
        }
        color.rgb = sum / 7.0;
    }
}
