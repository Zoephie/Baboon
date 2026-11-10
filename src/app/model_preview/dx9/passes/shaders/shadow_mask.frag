${version}in vec4 v_tc0;
uniform sampler2D u_shadow_map;
void main() {
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    if (place.z - ${bias} <= texture(u_shadow_map, place.xy).r) {
        discard;
    }
}
