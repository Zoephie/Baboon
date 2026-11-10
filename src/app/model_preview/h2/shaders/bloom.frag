
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_source;
uniform int u_mode;
uniform vec2 u_step;
uniform float u_scale;
uniform float u_threshold;
uniform float u_brightness;
void main() {
    if (u_mode == 0) {
        vec4 color = texture(u_source, v_uv);
        out_color = vec4(clamp(4.0 * (color.rgb - u_threshold), 0.0, 1.0), color.a * u_scale);
    } else if (u_mode == 1) {
        vec4 sum = texture(u_source, v_uv) * 0.2270270;
        const vec2 taps[4] = vec2[4](
            vec2(1.0, 0.1945946), vec2(2.0, 0.1216216),
            vec2(3.0, 0.0540541), vec2(4.0, 0.0162162));
        for (int tap = 0; tap < 4; tap++) {
            vec2 offset = u_step * taps[tap].x;
            sum += (texture(u_source, v_uv + offset) + texture(u_source, v_uv - offset)) * taps[tap].y;
        }
        out_color = sum;
    } else {
        // `pPixelShader_6`: twice the bright part and the self-bloom.
        vec4 bloom = texture(u_source, v_uv);
        out_color = vec4(2.0 * (bloom.rgb * u_brightness + bloom.a), 1.0);
    }
}
