#version 410 core
uniform sampler2D source_map;
uniform vec4 offsets;
out vec4 color;
void main() {
    vec2 uv = gl_FragCoord.xy / vec2(textureSize(source_map, 0));
    vec2 t0 = texture(source_map, uv + vec2(0.0, offsets.x)).xy;
    vec2 t1 = texture(source_map, uv + vec2(0.0, offsets.y)).xy;
    vec2 t2 = texture(source_map, uv + vec2(0.0, offsets.z)).xy;
    vec2 t3 = texture(source_map, uv + vec2(0.0, offsets.w)).xy;
    vec4 weights = vec4(2.0, 6.0, 6.0, 2.0) / 16.0;
    color = vec4(dot(vec4(t0.x, t1.x, t2.x, t3.x), weights), dot(vec4(t0.y, t1.y, t2.y, t3.y), weights), 0.0, 0.0);
}
