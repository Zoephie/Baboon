#version 410 core
// A tile's window of a picture held from the whole of it: what a snapshot's
// tile reads where the whole-picture draw would read `source` at its own
// place.
uniform sampler2D source;
uniform vec4 window;
uniform vec2 size;
out vec4 color;
void main() {
    color = texture(source, gl_FragCoord.xy / size * window.zw + window.xy);
}
