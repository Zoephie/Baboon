#version 410 core
uniform sampler2D depth;
uniform sampler2D mask;
uniform mat4 screen_to_world;
uniform vec3 eye;
uniform vec3 forward;
uniform vec2 planes;
uniform float falloff;
uniform vec3 ping;
uniform vec4 default_color;
uniform vec4 ping_color;
out vec4 color;
vec3 world_at(ivec2 at, float raw) {
    vec2 size = vec2(textureSize(depth, 0));
    vec4 place = screen_to_world * vec4((vec2(at) + 0.5) / size * 2.0 - 1.0, raw * 2.0 - 1.0, 1.0);
    return place.xyz / place.w;
}
float game_depth(ivec2 at) {
    float raw = texelFetch(depth, at, 0).r;
    if (raw >= 1.0) {
        return 0.0;
    }
    float along = dot(world_at(at, raw) - eye, forward);
    return planes.x * (planes.y - along) / (along * (planes.y - planes.x));
}
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float raw = texelFetch(depth, at, 0).r;
    vec2 place = gl_FragCoord.xy / vec2(textureSize(depth, 0));
    float cover = texture(mask, vec2(place.x, 1.0 - place.y)).r;
    if (raw >= 1.0 || cover <= 0.0) {
        discard;
    }
    float middle = game_depth(at);
    vec2 laplacian = vec2(
        game_depth(at + ivec2(1, 0)) + game_depth(at - ivec2(1, 0)) - 2.0 * middle,
        game_depth(at + ivec2(0, 1)) + game_depth(at - ivec2(0, 1)) - 2.0 * middle);
    float gradient = clamp(length(laplacian) / middle, 0.0, 1.0);
    if (gradient <= 1.0 / 255.0) {
        discard;
    }
    if (laplacian.x + laplacian.y > 0.0) {
        gradient *= ping_color.a;
    }
    float distance_away = length(world_at(at, raw) - eye);
    cover *= pow(2.0, -pow(distance_away * falloff, 8.0));
    float after = ping.x - distance_away;
    float boost = pow(clamp(1.0 + ping.z * after, 0.0, 1.0), 4.0) * step(distance_away, ping.x);
    cover *= step(distance_away, ping.y);
    color = vec4(gradient * (default_color.rgb + ping_color.rgb * boost), default_color.a) * cover;
}
