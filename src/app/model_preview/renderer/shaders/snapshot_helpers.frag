#version 410 core
// As egui's own painter draws: its colours and textures are both gamma,
// premultiplied.
uniform sampler2D image;
in vec2 v_uv;
in vec4 v_rgba;
out vec4 color;
void main() {
    color = v_rgba * texture(image, v_uv);
}
