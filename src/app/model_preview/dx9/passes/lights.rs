//! The dx9 passes' default lights, and the object light they read.

use super::globals::DEFAULT_DIRECT_VECTOR;

/// The sphere lights `render_lights` falls back on with nothing else lighting
/// the scene: a warm key and a cool fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Dx9Light {
    pub(crate) position: [f32; 3],
    pub(crate) color: [f32; 3],
    pub(crate) specular: [f32; 3],
    pub(crate) radius: f32,
}

pub(crate) const DEFAULT_LIGHTS: [Dx9Light; 2] = [
    Dx9Light {
        position: [2000.0, 2000.0, 2000.0],
        color: [0.9, 0.9, 0.8],
        specular: [0.9, 0.9, 0.8],
        radius: 40000.0,
    },
    Dx9Light {
        position: [-2000.0, -2000.0, 500.0],
        color: [0.2, 0.2, 0.3],
        specular: [0.2, 0.2, 0.3],
        radius: 40000.0,
    },
];

/// Stands in for the direct light on the object path, where a shadow leaves
/// the pass only its ambient, indirect and reflected light.
pub(crate) const SHADOWED_LIGHT: Dx9Light = Dx9Light {
    position: [0.0; 3],
    color: [0.0; 3],
    specular: [0.0; 3],
    radius: 1.0,
};

/// Which way `light` lies from the origin.
pub(crate) fn light_direction(light: &Dx9Light) -> [f32; 3] {
    let [x, y, z] = light.position;
    let length = (x * x + y * y + z * z).sqrt();
    [x / length, y / length, z / length]
}

/// Which way the object path's direct light lies from a surface. A pass
/// lights by the negated vector.
pub(crate) fn object_light_direction() -> [f32; 3] {
    let [x, y, z, _] = DEFAULT_DIRECT_VECTOR;
    let length = (x * x + y * y + z * z).sqrt();
    [-x / length, -y / length, -z / length]
}

/// Enough to clear a surface of its own shadow, as a share of the depth range.
pub(super) const SHADOW_BIAS: f32 = 0.004;
