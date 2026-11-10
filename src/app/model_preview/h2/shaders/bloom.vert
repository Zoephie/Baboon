
out vec2 v_uv;
void main() {
    v_uv = vec2(gl_VertexID == 1 ? 2.0 : 0.0, gl_VertexID == 2 ? 2.0 : 0.0);
    gl_Position = vec4(v_uv * 2.0 - 1.0, 0.0, 1.0);
}
