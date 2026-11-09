#version 330 core
uniform sampler2DArray u_render;
uniform float u_exposure;
// Halo 4 runs from its exposure to its self-illumination one by alpha;
// Reach's two are the same.
uniform float u_self_illum;
in vec3 v_texcoord;
in vec4 v_color;
out vec4 out_color;
void main() {
    vec4 texel = texture(u_render, v_texcoord, -0.5) * v_color;
    out_color = vec4(texel.rgb * mix(u_exposure, u_self_illum, texel.a), texel.a);
}
