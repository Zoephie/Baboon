#version 410 core
uniform sampler2D scene_depth;
uniform sampler2D normals;
uniform sampler2D variance;
uniform mat4 screen_to_world;
uniform mat4 shadow_view_projection;
uniform vec3 shadow_direction;
uniform float shadow_alpha;
uniform vec2 fade;
uniform vec3 vsm;
uniform float knockout;
uniform int normal_test;
out vec4 color;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(scene_depth, at, 0).r;
    if (depth >= 1.0) {
        discard;
    }
    vec4 world = screen_to_world * vec4(gl_FragCoord.xy, depth, 1.0);
    world /= world.w;
    vec4 clip = shadow_view_projection * vec4(world.xyz, 1.0);
    if (any(greaterThan(abs(clip.xyz), vec3(1.0)))) {
        discard;
    }
    vec3 shadow = vec3(clip.xy * 0.5 + 0.5, clip.z * 0.5 + 0.5);
    float falloff = max(shadow.z * fade.x + fade.y, 0.0);
    falloff = falloff * falloff / 2.25;
    float darkness = shadow_alpha * (1.0 - falloff);
    // `DecodeNormal`, into the space the normal buffer was written in.
    vec2 expanded = texelFetch(normals, at, 0).xy * 2.0 - 1.0;
    float f = dot(expanded, expanded);
    vec3 normal = vec3(expanded * sqrt(max(4.0 - 4.0 * f, 0.0)), 1.0 - 2.0 * f);
    float cosine = dot(normal, shadow_direction);
    if (normal_test != 0) {
        darkness *= clamp(0.65 + cosine * 5.0, 0.0, 1.0);
    }
    float darken = 1.0;
    if (darkness > 0.001) {
        shadow.z = min(shadow.z, 0.99);
        vec2 moments = texture(variance, shadow.xy).xy;
        float closer = 1.0;
        if (shadow.z > moments.x) {
            float spread = clamp(moments.y - moments.x * moments.x + vsm.x, 0.0, 1.0);
            float d2 = pow(shadow.z - moments.x, vsm.y);
            closer = pow(spread / (spread + d2), vsm.z);
        }
        darken = clamp(1.01 - darkness + closer * darkness, 0.0, 1.0);
        darken *= darken;
    }
    color = vec4(darken, knockout + darken, darken, 0.0);
}
