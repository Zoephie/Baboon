${prelude}${prt}invariant gl_Position;
out vec4 v_color0;
out vec4 v_color1;
out vec4 v_tc0;
out vec4 v_tc1;
out vec4 v_tc2;
out vec4 v_tc3;
out vec4 v_tc4;
out vec4 v_tc5;
out vec4 v_tc6;
out vec4 v_tc7;
out float v_fog;
uniform vec3 u_eye;
uniform vec2 u_view_size;
uniform vec3 u_view_right;
uniform vec3 u_view_up;
uniform vec3 u_view_back;
uniform mat4 u_shadow_matrix;
uniform vec4 u_texture_rows[${texture_rows}];
uniform vec4 u_tint_facing;
uniform vec4 u_tint_glancing;
uniform float u_reflection_scale;
uniform vec3 u_light_direct;
uniform vec3 u_light_indirect;
uniform vec3 u_light_position;
uniform float u_light_falloff;
uniform vec4 u_gel_rows[4];
uniform vec4 u_ce_point[${ce_point_rows}];
vec3 tangent_space(vec3 value, vec3 tangent, vec3 binormal, vec3 normal) {
    return vec3(dot(value, tangent), dot(value, binormal), dot(value, normal));
}
float row(int index, vec3 point) {
    return dot(u_texture_rows[index].xyz, point) + u_texture_rows[index].w;
}
vec3 ce_point_light(vec3 position, vec3 normal) {
    vec3 sum = vec3(0.0);
    for (int light = 0; light < 2; light++) {
        vec4 place = u_ce_point[light * 3];
        vec4 aim = u_ce_point[light * 3 + 1];
        vec4 color = u_ce_point[light * 3 + 2];
        vec3 to_light = place.xyz - position;
        float squared = dot(to_light, to_light);
        to_light /= sqrt(max(squared, 0.000001));
        float near = max(1.0 - squared * place.w, 0.0);
        float facing = clamp(dot(to_light, normal), 0.0, 1.0);
        float spot = max(dot(to_light, -aim.xyz) * aim.w + color.w, 0.0);
        // `model.vsh` caps only the second light's spot at 1.
        sum += near * facing * (light == 1 ? min(spot, 1.0) : spot) * color.rgb;
    }
    return sum;
}
void main() {
    vec3 position;
    vec3 normal_in;
    vec3 tangent;
    vec3 binormal;
    skinned_pose(position, normal_in, tangent, binormal);
    vec3 normal = length(normal_in) > 0.0001 ? normalize(normal_in) : vec3(0.0, 0.0, 1.0);
    vec3 uv = vec3(a_texcoord, 0.0);
    vec3 to_eye = u_eye - position;
    float facing = max(dot(normal, normalize(to_eye)), 0.0);
    gl_Position = clip_position(${drawn});
    v_color0 = u_tint_facing * facing + u_tint_glancing * (1.0 - facing);
    v_color1 = vec4(1.0);
    v_tc0 = vec4(0.0, 0.0, 0.0, 1.0);
    v_tc1 = v_tc0;
    v_tc2 = v_tc0;
    v_tc3 = v_tc0;
    v_tc4 = v_tc0;
    v_tc5 = v_tc0;
    v_tc6 = v_tc0;
    v_tc7 = vec4(0.0, 0.0, 0.0, 1.0);
    v_fog = 1.0;
    ${outputs}
    ${variant}
}
