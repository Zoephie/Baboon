#version 410 core
uniform sampler2D source;
uniform sampler2D added;
uniform int mode;
uniform vec2 target_texel;
uniform vec2 source_texel;
uniform vec2 curve;
uniform float scale;
out vec4 color;
vec2 place;
vec3 at(vec2 offset) {
    return texture(source, place + offset * source_texel).rgb;
}
void main() {
    place = gl_FragCoord.xy * target_texel;
    vec3 result;
    if (mode == 0) {
        // `downsample_4x4_block_bloom`. Its LDR pass takes the larger of the two,
        // and clamped colors never pass this, so it has nothing to add.
        vec3 mean = (at(vec2(-1.0, -1.0)) + at(vec2(1.0, -1.0))
            + at(vec2(-1.0, 1.0)) + at(vec2(1.0, 1.0))) / 4.0 + 0.00000001;
        float intensity = dot(mean, vec3(0.299, 0.587, 0.114));
        result = mean * max(intensity * curve.y, intensity - curve.x) / intensity;
    } else if (mode == 1) {
        // `downsample_4x4_gaussian`
        result = 0.0625 * (at(vec2(-2.0, -2.0)) + at(vec2(2.0, -2.0))
                + at(vec2(-2.0, 2.0)) + at(vec2(2.0, 2.0)))
            + 0.125 * (at(vec2(0.0, -2.0)) + at(vec2(-2.0, 0.0))
                + at(vec2(2.0, 0.0)) + at(vec2(0.0, 2.0)))
            + 0.25 * at(vec2(0.0));
    } else if (mode == 2) {
        // `blur_11_horizontal`
        result = (10.0 * at(vec2(-4.1, 0.5)) + 120.0 * at(vec2(-2.3, 0.5))
            + 252.0 * at(vec2(-0.5, 0.5)) + 120.0 * at(vec2(1.3, 0.5))
            + 10.0 * at(vec2(3.1, 0.5))) / 512.0;
    } else if (mode == 3) {
        // `kernel_5`, with `gaussian_blur_fixed`'s kernel.
        result = (at(vec2(0.5, -4.1)) * 0.01953125 + at(vec2(0.5, -2.3)) * 0.234375
            + at(vec2(0.5, -0.5)) * 0.4921875 + at(vec2(0.5, 1.3)) * 0.234375
            + at(vec2(0.5, 3.1)) * 0.01953125) * scale;
    } else {
        // `bloom_add_alpha1` and `add`, whose alpha is 1 here.
        result = scale * at(vec2(0.0)) + texture(added, place).rgb;
    }
    color = vec4(result, 1.0);
}
