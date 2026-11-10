#version 410 core
uniform sampler2D depth_map;
uniform vec4 gaussian;
out vec4 color;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float d[5];
    for (int tap = 0; tap < 5; tap++) {
        d[tap] = texelFetch(depth_map, clamp(at + ivec2(tap - 2, 0), ivec2(0), textureSize(depth_map, 0) - 1), 0).r;
    }
    vec4 weights = vec4(gaussian.x, gaussian.y, gaussian.z, gaussian.y);
    float z = dot(vec4(d[0], d[1], d[2], d[3]), weights) + d[4] * gaussian.x;
    float z2 = dot(vec4(d[0], d[1], d[2], d[3]) * vec4(d[0], d[1], d[2], d[3]), weights) + d[4] * d[4] * gaussian.x;
    color = vec4(z, z2, 0.0, 0.0);
}
