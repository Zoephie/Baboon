#version 330 core
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_sprite;
layout(location = 2) in vec2 a_next_sprite;
layout(location = 3) in vec2 a_billboard;
layout(location = 4) in vec4 a_color;
layout(location = 5) in vec3 a_extra;
layout(location = 6) in vec4 a_lean;
layout(location = 7) in vec3 a_normal;
out vec4 v_lean;
out vec3 v_normal;
out vec2 v_sprite;
out vec2 v_next_sprite;
out vec2 v_billboard;
out vec4 v_color;
out vec3 v_extra;
void main() {
    gl_Position = a_clip;
    v_sprite = a_sprite;
    v_next_sprite = a_next_sprite;
    v_billboard = a_billboard;
    v_color = a_color;
    v_extra = a_extra;
    v_lean = a_lean;
    v_normal = a_normal;
}
