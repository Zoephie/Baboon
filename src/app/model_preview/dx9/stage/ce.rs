//! The GLSL Halo CE's transparent stages draw with.

/// Point lights a model takes: position and inverse radius squared, direction
/// and spot A, color and spot B, for each of two.
pub(crate) const CE_POINT_LIGHT_ROWS: usize = 6;

/// The first map's cube coordinate, from `cube` and the map's transform, and
/// everything else as `CE_TRANSPARENT` has it.
pub(super) const CE_TRANSPARENT_CUBE: &str = "vec3 across = u_texture_rows[0].xyz;
    vec3 up = u_texture_rows[1].xyz;
    v_tc0 = vec4(dot(cube, across), dot(cube, up), dot(cube, cross(across, up)), 1.0);
    v_tc1.xy = vec2(row(2, uv), row(3, uv));
    v_tc2.xy = vec2(row(4, uv), row(5, uv));
    v_tc3.xy = vec2(row(6, uv), row(7, uv));
    float face = abs(dot(normal, u_view_back));
    v_color0 = vec4(0.0, 0.0, 0.0, u_texture_rows[8].x);
    v_color1 = vec4(vec3(face), 1.0 - face) * u_texture_rows[8].x;";

/// `transparent_generic.vsh` without its fog, whatever the first map does.
pub(super) const CE_TRANSPARENT: &str = "v_tc0.xy = vec2(row(0, uv), row(1, uv));
    v_tc1.xy = vec2(row(2, uv), row(3, uv));
    v_tc2.xy = vec2(row(4, uv), row(5, uv));
    v_tc3.xy = vec2(row(6, uv), row(7, uv));
    float face = abs(dot(normal, u_view_back));
    v_color0 = vec4(0.0, 0.0, 0.0, u_texture_rows[8].x);
    v_color1 = vec4(vec3(face), 1.0 - face) * u_texture_rows[8].x;";
