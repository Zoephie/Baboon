#version 330
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_uv;
out vec2 v_uv;
void main() {
    gl_Position = a_clip;
    v_uv = a_uv;
}
