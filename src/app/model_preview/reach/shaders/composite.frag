#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform sampler2D scene_depth;
uniform sampler2D noise;
uniform mat4 hue_saturation;
uniform float dark_multiplier;
uniform float view_scale;
uniform float gamma;
uniform vec4 noise_params;
uniform vec2 noise_offset;
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 combined = (accumulated.rgb + texture(bloom, place).rgb) * dark_multiplier;
    combined = pow(max(combined, vec3(0.0)), vec3(gamma));
    color.rgb = clamp((vec4(combined, 1.0) * hue_saturation).rgb, 0.0, 1.0);
    // `apply_noise`: the map tiled a texel a pixel (its 256 square in the
    // game's), point sampled, at this frame's offset.
    float shade = texture(noise, gl_FragCoord.xy / vec2(textureSize(noise, 0)) + noise_offset).z;
    vec2 noise = shade * noise_params.xy + noise_params.zw;
    color.rgb = color.rgb * noise.x + noise.y;
    // Reach's lit target keeps its anti-aliasing blur in alpha, not cover:
    // what the object drew covers what lies behind it.
    bool drawn = texture(scene_depth, place).r < 1.0;
    color.a = drawn ? 1.0 : accumulated.a;
}
