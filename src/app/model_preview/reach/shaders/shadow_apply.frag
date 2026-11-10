#version 410 core
uniform sampler2D scene_depth;
uniform sampler2D normals;
uniform sampler2D shadow_map;
uniform mat4 screen_to_world;
uniform mat4 shadow_view_projection;
uniform vec2 texel_offset;
uniform vec3 hull_center;
uniform vec3 hull_left;
uniform vec3 hull_up;
uniform vec3 shadow_direction;
uniform vec2 hull_scale;
uniform vec4 hull_planes[14];
uniform int filter_mode;
uniform float shadow_alpha;
out vec4 color;

float fetch(vec2 uv, vec2 offset, vec2 pixel) {
    return texture(shadow_map, uv + offset * pixel).r;
}

// `sample_percentage_closer_PCF_3x3_block` with `BILINEAR_SHADOWS`.
float bilinear(vec2 uv, float z, vec2 pixel) {
    vec4 blend;
    blend.xy = fract(uv / pixel);
    blend.zw = 1.0 - blend.xy;
    float sum =
        blend.z * blend.w * step(z, fetch(uv, vec2(-1.0, -1.0), pixel)) +
        blend.w * step(z, fetch(uv, vec2(0.0, -1.0), pixel)) +
        blend.x * blend.w * step(z, fetch(uv, vec2(1.0, -1.0), pixel)) +
        blend.z * step(z, fetch(uv, vec2(-1.0, 0.0), pixel)) +
        step(z, fetch(uv, vec2(0.0, 0.0), pixel)) +
        blend.x * step(z, fetch(uv, vec2(1.0, 0.0), pixel)) +
        blend.z * blend.y * step(z, fetch(uv, vec2(-1.0, 1.0), pixel)) +
        blend.y * step(z, fetch(uv, vec2(0.0, 1.0), pixel)) +
        blend.x * blend.y * step(z, fetch(uv, vec2(1.0, 1.0), pixel));
    return sum / 4.0;
}

// `sample_percentage_closer_PCF_5x5_block_predicated`: 4x4 points, a 3x3
// bilinear.
float fancy(vec2 uv, float z, vec2 pixel) {
    vec4 blend;
    blend.xy = fract(uv / pixel + 0.5);
    blend.zw = 1.0 - blend.xy;
    float offsets[4] = float[4](-1.5, -0.5, 0.5, 1.5);
    float columns[4] = float[4](blend.z, 1.0, 1.0, blend.x);
    float rows[4] = float[4](blend.w, 1.0, 1.0, blend.y);
    float sum = 0.0;
    for (int row = 0; row < 4; row++) {
        for (int column = 0; column < 4; column++) {
            sum += columns[column] * rows[row]
                * step(z, fetch(uv, vec2(offsets[column], offsets[row]), pixel));
        }
    }
    return sum / 9.0;
}

void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(scene_depth, at, 0).r;
    if (depth >= 1.0) {
        discard;
    }
    vec4 world = screen_to_world * vec4(gl_FragCoord.xy, depth, 1.0);
    world /= world.w;
    vec3 offset = world.xyz - hull_center;
    vec3 hull = vec3(dot(offset, shadow_direction) / hull_scale.x, dot(offset, hull_left) / hull_scale.y,
        dot(offset, hull_up) / hull_scale.y);
    for (int face = 0; face < 14; face++) {
        if (dot(hull_planes[face].xyz, hull) > hull_planes[face].w) {
            discard;
        }
    }
    vec4 clip = shadow_view_projection * vec4(world.xyz, 1.0);
    // The map was drawn half a texel over; the apply reads it without.
    vec2 uv = (clip.xy - texel_offset) * 0.5 + 0.5;
    float z = clip.z * 0.5 + 0.5;
    vec3 normal = texelFetch(normals, at, 0).xyz * 2.0 - 1.0;
    float cosine = dot(normal, -shadow_direction);
    float falloff = clamp(z * 2.0 - 1.0, 0.0, 1.0);
    falloff *= falloff;
    float darkness = shadow_alpha * (1.0 - falloff * falloff) * clamp(0.65 + cosine * 5.0, 0.0, 1.0);
    float darken = 1.0;
    if (darkness > 0.001) {
        vec2 pixel = 1.0 / vec2(textureSize(shadow_map, 0));
        float closer = filter_mode == 0 ? step(z, texture(shadow_map, uv).r)
            : filter_mode == 1 ? bilinear(uv, z, pixel) : fancy(uv, z, pixel);
        darken = clamp(1.01 - darkness + closer * darkness, 0.0, 1.0);
        darken *= darken;
    }
    color = vec4(darken, 0.0, 0.0, darken) * vec4(1.0, 1.0, 1.0, 1.0) + vec4(0.2, 0.2, 0.2, 1.0);
}
