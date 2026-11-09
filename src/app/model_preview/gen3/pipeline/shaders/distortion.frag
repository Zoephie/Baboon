#version 410 core
uniform sampler2D scene;
uniform sampler2D displacement;
out vec4 color;
void main() {
    vec2 lean = texelFetch(displacement, ivec2(gl_FragCoord.xy), 0).xy;
    if (lean == vec2(0.0)) {
        discard;
    }
    vec2 place = gl_FragCoord.xy / vec2(textureSize(scene, 0));
    color = texture(scene, place + vec2(lean.x, -lean.y));
}
