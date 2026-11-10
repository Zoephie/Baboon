#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform vec4 tone_curve_constants;
uniform mat4 hue_saturation;
uniform float contrast;
uniform float exposure;
uniform float view_scale;
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    // A wider lens frames the same picture smaller, about the middle.
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 blend = (accumulated.rgb + texture(bloom, place).rgb) * exposure;
    blend = (vec4(blend, 1.0) * hue_saturation).rgb;
    float luminance = dot(blend, vec3(0.333));
    if (luminance > 0.0) {
        blend *= pow(luminance, contrast) / luminance;
    }
    vec3 clamped = min(blend, tone_curve_constants.xxx);
    vec3 toned = ((clamped * tone_curve_constants.w + tone_curve_constants.z) * clamped
        + tone_curve_constants.y) * clamped;
    // The game's surfaces are sRGB, so this happens as it writes.
    toned = clamp(toned, 0.0, 1.0);
    color.rgb = mix(12.92 * toned, 1.055 * pow(toned, vec3(1.0 / 2.4)) - 0.055,
        step(0.0031308, toned));
    color.a = accumulated.a;
}
