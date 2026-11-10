${version}${precision}${attribute} vec3 a_position;
${attribute} vec3 a_normal;
${attribute} vec2 a_texcoord;
${attribute} vec3 a_tangent;
${attribute} vec3 a_binormal;
${attribute} vec4 a_node_indices;
${attribute} vec4 a_node_weights;
uniform vec3 u_center;
uniform float u_scale;
uniform vec2 u_angles;
uniform vec2 u_clip_scale;
uniform float u_depth_scale;
// Depth remap `z = t * x + y` across the near and far planes.
// See PREVIEW_RENDER_DISTANCE.
uniform vec2 u_depth_map;
// 1 / eye distance (scaled view units); 0 is orthographic.
uniform float u_perspective;
// Skinning: three vec4 rows of an affine matrix per bone,
// bind-relative (world * inverse_bind), so zero-weight vertices
// and unanimated draws stay exactly where the tag put them.
uniform float u_animated;
uniform vec4 u_bones[${bone_rows}];
${varying_out} vec2 v_uv;
${varying_out} vec3 v_normal;
${varying_out} vec3 v_tangent;
${varying_out} vec3 v_binormal;
vec3 rotate_view(vec3 value) {
float sy = sin(u_angles.x);
float cy = cos(u_angles.x);
float sp = sin(u_angles.y);
float cp = cos(u_angles.y);
vec3 yawed = vec3(value.x * cy - value.y * sy, value.x * sy + value.y * cy, value.z);
return vec3(yawed.x, yawed.y * cp - yawed.z * sp, yawed.y * sp + yawed.z * cp);
}
void skin_influence(float index_f, float weight, vec4 point_h,
inout vec3 skinned_pos, inout vec3 skinned_nrm,
inout vec3 skinned_tan, inout vec3 skinned_bin) {
if (weight <= 0.0) { return; }
int base = int(index_f + 0.5) * 3;
vec4 r0 = u_bones[base];
vec4 r1 = u_bones[base + 1];
vec4 r2 = u_bones[base + 2];
skinned_pos += weight * vec3(dot(r0, point_h), dot(r1, point_h), dot(r2, point_h));
skinned_nrm += weight * vec3(dot(r0.xyz, a_normal), dot(r1.xyz, a_normal), dot(r2.xyz, a_normal));
skinned_tan += weight * vec3(dot(r0.xyz, a_tangent), dot(r1.xyz, a_tangent), dot(r2.xyz, a_tangent));
skinned_bin += weight * vec3(dot(r0.xyz, a_binormal), dot(r1.xyz, a_binormal), dot(r2.xyz, a_binormal));
}
void skinned_pose(out vec3 position, out vec3 normal_in, out vec3 tangent_in, out vec3 binormal_in) {
position = a_position;
normal_in = a_normal;
tangent_in = a_tangent;
binormal_in = a_binormal;
float weight_total = a_node_weights.x + a_node_weights.y + a_node_weights.z + a_node_weights.w;
if (u_animated > 0.5 && weight_total > 0.0001) {
vec4 point_h = vec4(a_position, 1.0);
vec3 sp = vec3(0.0);
vec3 sn = vec3(0.0);
vec3 st = vec3(0.0);
vec3 sb = vec3(0.0);
skin_influence(a_node_indices.x, a_node_weights.x, point_h, sp, sn, st, sb);
skin_influence(a_node_indices.y, a_node_weights.y, point_h, sp, sn, st, sb);
skin_influence(a_node_indices.z, a_node_weights.z, point_h, sp, sn, st, sb);
skin_influence(a_node_indices.w, a_node_weights.w, point_h, sp, sn, st, sb);
position = sp / weight_total;
normal_in = sn;
tangent_in = st;
binormal_in = sb;
}
}
vec4 clip_position(vec3 position) {
vec3 view = rotate_view((position - u_center) * u_scale);
// Perspective: the eye sits along -Y so w grows with depth;
// at the focus plane (view.y = 0) w is exactly 1 and the
// framing matches the orthographic view. See the derivation
// on PERSPECTIVE_FOV_Y.
float w = 1.0 + view.y * u_perspective;
float z_clip = view.y * u_depth_scale * u_depth_map.x + u_depth_map.y;
return vec4(view.x * u_clip_scale.x, view.z * u_clip_scale.y, z_clip, w);
}
void main() {
vec3 position;
vec3 normal_in;
vec3 tangent_in;
vec3 binormal_in;
skinned_pose(position, normal_in, tangent_in, binormal_in);
gl_Position = clip_position(position);
vec3 source_normal = length(normal_in) > 0.0001 ? normalize(normal_in) : vec3(0.0, 0.0, 1.0);
v_normal = rotate_view(source_normal);
v_tangent = rotate_view(tangent_in);
v_binormal = rotate_view(binormal_in);
v_uv = a_texcoord;
}
