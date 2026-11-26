#version 410 core
// egui's meshes (the viewport's markers, armature, bounding sphere), in
// points over the viewport, onto the snapshot.
uniform vec2 screen_points;
layout(location = 0) in vec2 position;
layout(location = 1) in vec2 uv;
layout(location = 2) in vec4 rgba;
out vec2 v_uv;
out vec4 v_rgba;
void main() {
    gl_Position = vec4(2.0 * position.x / screen_points.x - 1.0,
                       1.0 - 2.0 * position.y / screen_points.y, 0.0, 1.0);
    v_uv = uv;
    v_rgba = rgba;
}
