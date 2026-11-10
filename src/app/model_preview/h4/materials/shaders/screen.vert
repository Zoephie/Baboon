#version 410 core
out vec2 v_TEXCOORD0;
void main() {
    vec2 corner = vec2(gl_VertexID & 1, gl_VertexID >> 1) * 4.0 - 1.0;
    gl_Position = vec4(corner, 0.0, 1.0);
    v_TEXCOORD0 = corner * 0.5 + 0.5;
}
