#version 330 core
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec3 a_texcoord;
layout(location = 2) in vec4 a_color;
out vec3 v_texcoord;
out vec4 v_color;
void main() {
    gl_Position = a_clip;
    v_texcoord = a_texcoord;
    v_color = a_color;
}
