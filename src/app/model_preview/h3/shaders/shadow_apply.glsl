in vec4 v_tc0;
uniform sampler2D u_shadow_map;
uniform sampler2D u_normal_target;
uniform vec3 u_shadow_toward;
uniform float u_shadow_size;
uniform float u_shadow_alpha;
out vec4 color;
float closer(ivec2 texel, ivec2 offset, float depth) {
    return step(depth, texelFetch(u_shadow_map, texel + offset, 0).r);
}
void main() {
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    vec3 normal = texelFetch(u_normal_target, ivec2(gl_FragCoord.xy), 0).xyz * 2.0 - 1.0;
    float cosine = dot(normal, u_shadow_toward);
    float falloff = clamp(place.z * 2.0 - 1.0, 0.0, 1.0);
    falloff *= falloff;
    float darkness = u_shadow_alpha * (1.0 - falloff * falloff) * clamp(cosine, 0.0, 1.0);
    float darken = 1.0;
    vec2 pixel = place.xy * u_shadow_size;
    if (darkness > 0.001 && all(greaterThan(pixel, vec2(1.0))) && all(lessThan(pixel, vec2(u_shadow_size - 2.0)))) {
        ivec2 t = ivec2(floor(pixel));
        vec2 f = fract(pixel);
        vec2 r = 1.0 - f;
        float lit = r.x * r.y * closer(t, ivec2(-1, -1), place.z)
            + r.y * closer(t, ivec2(0, -1), place.z)
            + f.x * r.y * closer(t, ivec2(1, -1), place.z)
            + r.x * closer(t, ivec2(-1, 0), place.z)
            + closer(t, ivec2(0, 0), place.z)
            + f.x * closer(t, ivec2(1, 0), place.z)
            + r.x * f.y * closer(t, ivec2(-1, 1), place.z)
            + f.y * closer(t, ivec2(0, 1), place.z)
            + f.x * f.y * closer(t, ivec2(1, 1), place.z);
        lit *= 0.25;
        darken = clamp(1.01 - darkness + lit * darkness, 0.0, 1.0);
        darken *= darken;
    }
    color = vec4(0.0, 0.0, 0.0, darken);
}
