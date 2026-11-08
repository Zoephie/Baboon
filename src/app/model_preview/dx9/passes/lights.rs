//! Halo 2's default lights, and the object and PRT lights its passes read.

use super::*;

/// The sphere lights `render_lights` falls back on with nothing else lighting
/// the scene: a warm key and a cool fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2DefaultLight {
    pub(crate) position: [f32; 3],
    pub(crate) color: [f32; 3],
    pub(crate) specular: [f32; 3],
    pub(crate) radius: f32,
}

pub(crate) const H2_DEFAULT_LIGHTS: [H2DefaultLight; 2] = [
    H2DefaultLight {
        position: [2000.0, 2000.0, 2000.0],
        color: [0.9, 0.9, 0.8],
        specular: [0.9, 0.9, 0.8],
        radius: 40000.0,
    },
    H2DefaultLight {
        position: [-2000.0, -2000.0, 500.0],
        color: [0.2, 0.2, 0.3],
        specular: [0.2, 0.2, 0.3],
        radius: 40000.0,
    },
];

/// Stands in for the direct light on the object path, where a shadow leaves
/// the pass only its ambient, indirect and reflected light.
pub(crate) const H2_SHADOWED_LIGHT: H2DefaultLight =
    H2DefaultLight { position: [0.0; 3], color: [0.0; 3], specular: [0.0; 3], radius: 1.0 };

/// Which way the object path's direct light lies from a surface. A pass
/// lights by the negated vector.
pub(crate) fn h2_object_light_direction() -> [f32; 3] {
    let [x, y, z, _] = DEFAULT_DIRECT_VECTOR;
    let length = (x * x + y * y + z * z).sqrt();
    [-x / length, -y / length, -z / length]
}

/// The object path's lighting as a PRT pass takes it: the indirect light,
/// then the direct one from `direction`. The game samples a level's probes
/// here, which a preview stands in for with the same colors the other passes
/// light by. The indirect light alone is what a shadow leaves.
pub(crate) fn h2_prt_object_lights(direction: [f32; 3]) -> ([H2PrtLight; 2], [f32; 3]) {
    let channels = |[r, g, b, _]: [f32; 4]| [r, g, b];
    let [x, y, z, _] = DEFAULT_INDIRECT_VECTOR;
    let lights = [
        H2PrtLight { direction: [-x, -y, -z], color: channels(DEFAULT_INDIRECT_COLOR) },
        H2PrtLight { direction, color: channels(DEFAULT_DIRECT_COLOR) },
    ];
    (lights, channels(DEFAULT_AMBIENT))
}

/// Enough to clear a surface of its own shadow, as a share of the depth range.
pub(super) const H2_SHADOW_BIAS: f32 = 0.004;

/// What a model's `sh prt` passes are lit by. The game samples the level's
/// light probes here, and with none it falls back on a stand-in from straight
/// below; a preview uses its own lights instead.
pub(crate) fn h2_prt_lights() -> [H2PrtLight; 2] {
    H2_DEFAULT_LIGHTS.map(|light| {
        let [x, y, z] = light.position;
        let length = (x * x + y * y + z * z).sqrt();
        H2PrtLight { direction: [x / length, y / length, z / length], color: light.color }
    })
}
