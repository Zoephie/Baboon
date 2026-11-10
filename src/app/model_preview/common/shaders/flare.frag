#version 330
in vec2 v_uv;
uniform sampler2D u_map;
uniform vec4 u_color;
uniform float u_highlight;
uniform vec2 u_tint;
uniform vec2 u_circular;
uniform float u_fade;
uniform vec4 u_depth;
uniform sampler2D u_scene_depth;
out vec4 out_color;
${view_depth}void main() {
    vec4 map = texture(u_map, v_uv);
    if (u_circular.x > 0.0) {
        // Reach's light volume `circular` albedo: a disc, and no map.
        vec2 delta = 2.0 * v_uv - 1.0;
        float disc = pow(clamp(u_circular.x - u_circular.x * dot(delta, delta), 0.0, 1.0), u_circular.y);
        map = vec4(vec3(disc), 1.0);
    }
    if (u_fade > 0.0) {
        // Reach's light volume depth fade, as the particles'.
        float scene = texelFetch(u_scene_depth, ivec2(gl_FragCoord.xy), 0).r;
        float behind = (view_depth(scene) - view_depth(gl_FragCoord.z)) / u_depth.w;
        map.a *= clamp(behind / u_fade, 0.0, 1.0);
    }
    if (u_tint.y > 0.0) {
        vec4 lifted = pow(map, vec4(u_tint.y)) * map;
        out_color = vec4((lifted.rgb * u_tint.x + map.rgb * u_color.rgb) * u_color.a, 1.0);
    } else if (u_highlight < 0.0) {
        out_color = vec4(u_color.rgb * map.rgb, u_color.a * map.a);
    } else {
        // The pixel shader of `sub_92E4A0`, read from its bytecode.
        vec3 square = map.rgb * map.rgb;
        vec3 sixth = square * square * square;
        out_color = vec4((1.0 - u_color.rgb) * u_highlight * sixth + map.rgb * u_color.rgb, u_color.a);
    }
}
