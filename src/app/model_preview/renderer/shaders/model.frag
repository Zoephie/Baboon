${version}${precision}${varying_in} vec2 v_uv;
${varying_in} vec3 v_normal;
${varying_in} vec3 v_tangent;
${varying_in} vec3 v_binormal;
uniform sampler2D u_tex_base;
uniform sampler2D u_tex_detail;
uniform sampler2D u_tex_bump;
uniform sampler2D u_tex_bump_detail;
uniform sampler2D u_tex_alpha;
uniform sampler2D u_tex_multipurpose;
// Which slots this material bound: 0-3 in `u_have_a`, 4-5 in
// `u_have_b.xy`. Each slot's UV multiplier is a per-axis pair:
// base and detail in `_a`, bump and bump detail in `_b`, alpha
// and multipurpose in `_c`.
uniform vec4 u_have_a;
uniform vec4 u_have_b;
uniform vec4 u_uv_scale_a;
uniform vec4 u_uv_scale_b;
uniform vec4 u_uv_scale_c;
// Detail combine: x the function (0 the Halo 3 linear-space
// biased multiply; 1-3 Halo CE's biased multiply, multiply and
// biased add, on stored values), y the CE detail mask (0 none),
// z the Xbox multipurpose channel order.
uniform vec4 u_detail;
uniform vec3 u_base_color;
uniform float u_unlit;
uniform float u_shaded;
${fragment_output}
vec3 to_linear(vec3 c) { return pow(c, vec3(2.2)); }
vec3 to_srgb(vec3 c) { return pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.2)); }
void main() {
vec3 albedo = u_base_color;
bool shaded = u_shaded > 0.5;
if (shaded) {
// Alpha test first: a discarded fragment costs nothing else.
// This is the ONLY thing that discards. A base map's alpha
// is not opacity in Halo — it carries a mask, most often
// specular — so treating it as coverage punched holes
// through every character whose diffuse had a dark mask.
if (u_have_b.x > 0.5 && ${sample}(u_tex_alpha, v_uv * u_uv_scale_c.xy).a < 0.5) discard;
if (u_have_a.x > 0.5) {
albedo = to_linear(${sample}(u_tex_base, v_uv * u_uv_scale_a.xy).rgb);
}
// Halo detail maps are grey-centred and modulate: mid-grey
// leaves the base untouched, darker and lighter push it.
// They tile — commonly sixteen times — so the scale matters.
if (u_have_a.y > 0.5) {
vec3 detail = ${sample}(u_tex_detail, v_uv * u_uv_scale_a.zw).rgb;
if (u_detail.x < 0.5) {
albedo = clamp(albedo * to_linear(detail) * 2.0, 0.0, 1.0);
} else {
// Halo CE's ShaderModel() (fx/model_common.h): the
// mask lerps the detail toward its function's
// neutral value, then combines with the base.
float mask = 1.0;
if (u_detail.y > 0.5 && u_have_b.y > 0.5) {
vec4 mp = ${sample}(u_tex_multipurpose, v_uv * u_uv_scale_c.zw);
if (u_detail.z > 0.5) mp = mp.agrb;
float k = u_detail.y;
float channel = k < 2.5 ? mp.b : (k < 4.5 ? mp.g : (k < 6.5 ? mp.a : mp.r));
mask = mod(k, 2.0) > 0.5 ? 1.0 - channel : channel;
}
vec3 neutral = u_detail.x > 1.5 && u_detail.x < 2.5 ? vec3(1.0) : vec3(0.5);
vec3 d = mix(neutral, detail, mask);
vec3 base = to_srgb(albedo);
vec3 combined = u_detail.x < 1.5 ? base * d * 2.0
: (u_detail.x < 2.5 ? base * d : base + 2.0 * d - 1.0);
albedo = to_linear(clamp(combined, 0.0, 1.0));
}
}
}
vec3 normal = normalize(v_normal);
if (shaded && (u_have_a.z > 0.5 || u_have_a.w > 0.5)) {
// Tangent space, using the frame the tag authored rather
// than a reconstructed one — mirrored UV islands need the
// binormal's own sign to come out the right way round.
vec3 t = normalize(v_tangent);
vec3 b = normalize(v_binormal);
if (length(v_tangent) > 0.0001 && length(v_binormal) > 0.0001) {
// The engine's own unpack, verbatim from the kit's
// source (rasterizer/hlsl/bump_mapping.fx): X and Y
// come off the texture through BUMP_CONVERT — byte
// 128 is exactly flat — Z is ALWAYS reconstructed
// rather than sampled (the compressed formats carry
// no trustworthy blue), and the components go into
// the authored tangent frame with no negation.
vec3 tn = vec3(0.0, 0.0, 1.0);
if (u_have_a.z > 0.5) {
vec2 sampled = ${sample}(u_tex_bump, v_uv * u_uv_scale_b.xy).xy * (255.0 / 127.0) - (128.0 / 127.0);
tn = vec3(sampled, sqrt(1.0 - min(dot(sampled, sampled), 1.0)));
}
// A detail normal tiles far finer than the base one
// and adds its tangent-plane components over it —
// the engine's stock combine — staying stable when
// either map is flat.
if (u_have_a.w > 0.5) {
vec2 dn = ${sample}(u_tex_bump_detail, v_uv * u_uv_scale_b.zw).xy * (255.0 / 127.0) - (128.0 / 127.0);
tn = vec3(tn.xy + dn, tn.z);
}
tn = normalize(tn);
normal = normalize(t * tn.x + b * tn.y + normal * tn.z);
}
}
// The rig this preview has always used — a key, a fill, a rim
// and an overhead term, in view space so they follow the camera.
// Neutral on purpose: the maps supply the colour, and anything
// that tried to supply more needed a scene this preview has not
// got.
float key = max(dot(normal, normalize(vec3(-0.35, -0.55, 0.76))), 0.0);
float fill = max(dot(normal, normalize(vec3(0.72, 0.22, 0.36))), 0.0);
float rim = pow(clamp(1.0 - abs(normal.y), 0.0, 1.0), 2.0);
float overhead = clamp(normal.z * 0.5 + 0.5, 0.0, 1.0);
float shade = clamp(0.42 + key * 0.46 + fill * 0.16 + rim * 0.10 + overhead * 0.08, 0.32, 1.22);
vec3 lit = albedo * shade + vec3(key * key * (22.0 / 255.0));
vec3 flat_color = shaded ? to_srgb(albedo) : u_base_color;
vec3 result = mix(to_srgb(lit), flat_color, u_unlit);
${output_name} = vec4(result, 1.0);
}
