#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform sampler2D scene_depth;
uniform sampler3D grading;
uniform vec3 grading_scale;
uniform vec3 grading_offset;
uniform float dark_multiplier;
uniform float view_scale;
uniform float filmic[5];
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 combined = max(accumulated.rgb, vec3(0.0)) * dark_multiplier + texture(bloom, place).rgb;
    combined = combined * (combined * filmic[0] + filmic[1]) / (combined * (combined * filmic[2] + filmic[3]) + filmic[4]);
    vec3 encoded = sqrt(max(combined, vec3(0.0)));
    // `apply_color_grading` reads level 0 (`sample3DLOD`). An implicit level
    // follows the lookup coordinate's change across each 2×2 block, which
    // jumps at a silhouette, and drew a grey rim from the lookup's small mips.
    color.rgb = textureLod(grading, encoded * grading_scale + grading_offset, 0.0).rgb;
    bool drawn = texture(scene_depth, place).r < 1.0;
    color.a = drawn ? 1.0 : accumulated.a;
}
