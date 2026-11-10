#version 330
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec2 a_unit;
layout(location = 3) in vec4 a_color;
out vec2 v_uv;
out vec2 v_unit;
out vec4 v_clip;
out vec4 v_color;
void main() {
    gl_Position = a_clip;
    v_uv = a_uv;
    v_unit = a_unit;
    v_clip = a_clip;
    v_color = a_color;
}
