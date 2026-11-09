#version 330 core
in vec2 v_sprite;
in vec2 v_next_sprite;
in vec2 v_billboard;
in vec4 v_color;
in vec3 v_extra;
in vec4 v_lean;
in vec3 v_normal;
uniform bool u_lit;
uniform bool u_alpha_test;
uniform bool u_premultiplied;
uniform vec4 u_lighting[10];
uniform float u_distortion;
uniform bool u_unsigned;
uniform sampler2D u_base;
uniform sampler2D u_alpha;
uniform sampler2D u_palette;
uniform sampler2D u_base2;
uniform vec4 u_base_xform;
uniform vec4 u_base2_xform;
uniform vec3 u_tint;
uniform vec2 u_modulation;
uniform int u_albedo;
uniform bool u_black_point;
uniform bool u_multiply;
uniform vec3 u_add;
uniform float u_fade;
uniform vec4 u_depth;
uniform sampler2D u_scene_depth;
uniform vec4 u_screen;
uniform bool u_smooth;
uniform vec3 u_reach_light[3];
uniform vec2 u_contrast;
uniform float u_sphere_warp;
uniform float u_palette_shift;
uniform bool u_plasma_second_sprite;
uniform bool u_unit_particle_alpha;
out vec4 out_color;
${view_depth}// `other` is the second sprite texcoord, which only Reach's plasmas read.
vec4 albedo(vec2 sprite, vec2 other, float particle_alpha, float depth_fade) {
    float external_alpha = particle_alpha * depth_fade;
    if (u_albedo == ${albedo_diffuse_modulated}) {
        vec4 diffuse = texture(u_base, sprite);
        float brightness = dot(diffuse.rgb, diffuse.rgb) / sqrt(3.0);
        diffuse.rgb *= mix(u_tint, vec3(1.0), u_modulation.x * brightness);
        return diffuse;
    }
    if (u_albedo == ${albedo_palette_glow}) {
        float ramp = texture(u_base, sprite).g;
        float glow = v_extra.y * ramp;
        return vec4(clamp(u_tint * glow * pow(2.0, glow * 6.0), 0.0, 1.0), ramp);
    }
    if (u_albedo == ${albedo_palette_plasma} || u_albedo == ${albedo_palette_2d_plasma}) {
        vec2 second = u_plasma_second_sprite ? other : sprite;
        float noise_a = texture(u_base, sprite * u_base_xform.xy + u_base_xform.zw).r;
        float noise_b = texture(u_base2, second * u_base2_xform.xy + u_base2_xform.zw).r;
        float alpha = texture(u_alpha, v_billboard).a;
        if (u_albedo == ${albedo_palette_2d_plasma}) {
            // Reach's `palettized_2d_plasma`: no alpha term but the palette
            // shift's, and the depth fade picks the palette's row.
            float index = clamp(abs(noise_a - noise_b), 0.0, 1.0);
            index = clamp(index + (1.0 - alpha * particle_alpha) * u_palette_shift, 0.0, 1.0);
            return vec4(texture(u_palette, vec2(index, depth_fade)).rgb, alpha);
        }
        float index = clamp(abs(noise_a - noise_b) + (1.0 - alpha * external_alpha) * u_modulation.y, 0.0, 1.0);
        return vec4(texture(u_palette, vec2(index, v_extra.y)).rgb, alpha);
    }
    vec4 base = texture(u_base, sprite);
    vec4 tone = base;
    float row = v_extra.y;
    // The palette options; Halo 2's tints read the base map alone.
    if (u_albedo >= ${albedo_palette} && u_albedo <= ${albedo_palette_sprite_alpha}) {
        // Reach's `palette_shift` moves the index by what has faded.
        float shifted = clamp(base.r + (1.0 - depth_fade * particle_alpha) * u_palette_shift, 0.0, 1.0);
        float index = u_palette_shift == 0.0 ? base.r : shifted;
        tone = texture(u_palette, vec2(index, row));
    }
    if (u_albedo == ${albedo_diffuse_billboard_alpha} || u_albedo == ${albedo_palette_billboard_alpha}) {
        tone.a = texture(u_alpha, v_billboard).a;
    } else if (u_albedo == ${albedo_diffuse_sprite_alpha} || u_albedo == ${albedo_palette_sprite_alpha}) {
        tone.a = texture(u_alpha, sprite).a;
    }
    return tone;
}
void main() {
    if (u_albedo == ${albedo_h2_distortion}) {
        // `effect_shader_alphablend_distortion_0.psh` (U is the decode's green,
        // V its red). MCC makes `v8u8` an `R8G8_SNORM` texture (`dword_EBC5B8`),
        // so the shader's unsigned-to-signed `bump - round(bump)` runs on bytes
        // already signed, and folds the strong ones back towards nothing.
        vec2 signed_byte = round(texture(u_base, v_sprite).gr * 127.0);
        vec2 snorm = max(signed_byte / 127.0, -1.0);
        vec2 bump = snorm - round(snorm);
        vec2 screen = (gl_FragCoord.xy - u_screen.xy) / u_screen.zw;
        // Its screen v runs down the frame, this one's up.
        vec2 read = screen + u_modulation.x / 64.0 * vec2(bump.x, -bump.y);
        if (v_color.a <= 0.0) {
            discard;
        }
        out_color = vec4(texture(u_palette, read).rgb * v_color.rgb, v_color.a);
        return;
    }
    float depth_fade = 1.0;
    if (u_fade > 0.0) {
        float scene = texelFetch(u_scene_depth, ivec2(gl_FragCoord.xy), 0).r;
        float behind = (view_depth(scene) - view_depth(gl_FragCoord.z)) / u_depth.w;
        depth_fade = clamp(behind / u_fade, 0.0, 1.0);
    }
    // Reach's `calc_warp` `sphere`: both sprites pushed out toward the rim.
    vec2 rim = v_billboard * 2.0 - 1.0;
    vec2 warp = u_sphere_warp == 0.0 ? vec2(0.0) : rim * dot(rim, rim) * dot(rim, rim) * u_sphere_warp;
    vec2 sprite = v_sprite + warp;
    vec2 next_sprite = v_next_sprite + warp;
    // Reach's beams sample with a `particle_alpha` of 1.
    float particle_alpha = u_unit_particle_alpha ? 1.0 : v_color.a;
    vec4 color = mix(
        albedo(sprite, next_sprite, particle_alpha, depth_fade),
        albedo(next_sprite, sprite, particle_alpha, depth_fade),
        v_extra.x
    );
    if (u_distortion != 0.0) {
        // `compute_normalized_distortion`, in the frame's own widths and heights.
        vec2 map = u_unsigned ? (color.xy * 255.0 - 128.0) / 127.0 : color.xy;
        vec2 push = vec2(map.x, -map.y) * v_color.a * depth_fade;
        out_color = vec4(vec2(dot(v_lean.xy, push), dot(v_lean.zw, push)) * u_distortion / 32.0, 0.0, 1.0);
        return;
    }
    color.a *= depth_fade;
    if (u_black_point) {
        float black = v_extra.z;
        float middle = (black + 1.0) * 0.5;
        color.a = middle * clamp((color.a - black) / (middle - black), 0.0, 1.0)
            + clamp(color.a - middle, 0.0, 1.0);
    }
    if (u_alpha_test) {
        if (color.a < 0.5) {
            discard;
        }
        color.a = 1.0;
    }
    if (u_albedo == ${albedo_h2_tint}) {
        vec3 fourth = color.rgb * color.rgb;
        fourth *= fourth;
        vec3 toward = mix(fourth, color.rgb, v_color.rgb);
        color.rgb = mix(toward, color.rgb * v_color.rgb, u_modulation.x);
        color.a *= v_color.a;
    } else if (u_albedo == ${albedo_h2_multiply_add}) {
        color.rgb *= v_color.rgb * v_color.a;
        color.a *= v_color.a;
    } else {
        color *= v_color;
    }
    if (u_lit) {
        vec3 ambient = 0.886227 * u_lighting[0].rgb;
        vec3 normal = normalize(v_normal);
        vec3 a = normal * normal.yzx;
        vec4 b = vec4(normal * normal, 1.0 / 3.0);
        vec3 first = vec3(dot(normal, u_lighting[1].rgb), dot(normal, u_lighting[2].rgb), dot(normal, u_lighting[3].rgb));
        vec3 second = vec3(dot(a, u_lighting[4].rgb), dot(a, u_lighting[5].rgb), dot(a, u_lighting[6].rgb));
        vec3 third = vec3(dot(b, u_lighting[7]), dot(b, u_lighting[8]), dot(b, u_lighting[9]));
        vec3 full = ambient - 2.0 * 0.511664 * first - 2.0 * 0.429043 * second - 0.429043 * third;
        color.rgb *= mix(ambient, full, color.a) / 3.1415926535;
    }
    if (u_smooth) {
        // Reach's `per_pixel_smooth`: from the probe's dark to its bright.
        float cosine = dot(normalize(v_normal), u_reach_light[0]);
        float toward = pow(clamp(cosine * u_contrast.x + u_contrast.y, 0.0, 1.0), 3.0);
        color.rgb *= mix(u_reach_light[2], u_reach_light[1], toward);
    }
    if (u_multiply) {
        color.rgb = mix(vec3(1.0), color.rgb, color.a);
    } else {
        color.rgb += u_add;
    }
    // `convert_to_render_target_premultiplied_alpha`.
    if (u_premultiplied && u_albedo != ${albedo_h2_multiply_add}) {
        color.rgb *= color.a;
    }
    out_color = color;
}
