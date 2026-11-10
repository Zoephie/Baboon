${version}in vec4 v_tc3;
in vec4 v_color1;
uniform sampler2D u_gel_map;
out vec4 color;
void main() {
    color = vec4(1.0, 1.0, 1.0, v_color1.r > 0.5 ? textureProj(u_gel_map, v_tc3).b : 0.0);
}
