#version 410 core
// The snapshot's matte: the frame drawn on black and on white, each pixel
// `C + T * background`, into straight-alpha colour with no background in it.
// `C` is the black render; `T` is how much the white one shows more.
// Light that adds without covering (bloom, additive effects) counts as cover
// as far as it is bright, so it survives past the model's outline. Rows are
// written top first, as an image is read.
uniform sampler2D black;
uniform sampler2D white;
out vec4 color;
void main() {
    ivec2 size = textureSize(black, 0);
    ivec2 at = ivec2(gl_FragCoord.xy);
    at.y = size.y - 1 - at.y;
    vec3 c = max(texelFetch(black, at, 0).rgb, vec3(0.0));
    vec3 t = clamp(texelFetch(white, at, 0).rgb - c, vec3(0.0), vec3(1.0));
    float cover = 1.0 - (t.r + t.g + t.b) / 3.0;
    float alpha = clamp(max(cover, max(c.r, max(c.g, c.b))), 0.0, 1.0);
    // Nothing a byte can show: no colour left behind in a clear pixel.
    if (alpha < 0.5 / 255.0) {
        color = vec4(0.0);
        return;
    }
    color = vec4(clamp(c / alpha, vec3(0.0), vec3(1.0)), alpha);
}
