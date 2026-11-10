#version 410 core
uniform sampler2D picture;
uniform vec2 spread;
uniform float cover;
out vec4 color;
void main() {
    vec2 pixel = 1.0 / vec2(textureSize(picture, 0));
    vec2 place = gl_FragCoord.xy * pixel;
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int y = -8; y <= 8; y++) {
        for (int x = -8; x <= 8; x++) {
            vec2 at = vec2(x, y) / 4.0;
            float weight = exp(-0.5 * dot(at, at));
            sum += texture(picture, place + at * spread * pixel * 8.0).rgb * weight;
            total += weight;
        }
    }
    color = vec4(sum / total, cover);
}
