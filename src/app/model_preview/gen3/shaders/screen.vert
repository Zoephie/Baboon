#version 410 core
out vec4 v_TEXCOORD0;
void main() {
    vec2 corner = vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0;
    gl_Position = vec4(corner, 0.0, 1.0);
    vec2 uv = vec2(corner.x * 0.5 + 0.5, 0.5 - corner.y * 0.5);
    v_TEXCOORD0 = vec4(uv, uv);
}
