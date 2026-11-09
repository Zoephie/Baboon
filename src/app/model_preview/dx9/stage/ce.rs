//! The GLSL Halo CE's transparent stages draw with.

/// Point lights a model takes: position and inverse radius squared, direction
/// and spot A, color and spot B, for each of two.
pub(crate) const CE_POINT_LIGHT_ROWS: usize = 6;

/// The first map's cube coordinate, from `cube` and the map's transform, and
/// everything else as `CE_TRANSPARENT` has it.
pub(super) const CE_TRANSPARENT_CUBE: &str = include_str!("shaders/ce_transparent_cube.glsl");

/// `transparent_generic.vsh` without its fog, whatever the first map does.
pub(super) const CE_TRANSPARENT: &str = include_str!("shaders/ce_transparent.glsl");
