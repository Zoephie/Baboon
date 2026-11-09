//! Reach's light volume level of detail and precompiled profiles, which Halo 4 also reads.

use super::*;

/// Reach's `LOD in distance` and the rest, as `postprocess` (tool
/// 0x140DD0FA0) leaves them, for `calc_lod_amount`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ReachVolumeLod {
    pub(super) near: f32,
    pub(super) feather_in: f32,
    pub(super) inverse_in: f32,
    pub(super) far: f32,
    pub(super) feather_out: f32,
    pub(super) inverse_out: f32,
    /// `lod always 1.0`, which with `lod enabled` draws nothing.
    pub(super) always_one: bool,
}

impl ReachVolumeLod {
    /// With `lod enabled` only; the inverses are the postprocess's, which a
    /// tag on disk holds 0 for.
    pub(super) fn read(element: TagStruct<'_>, flagged: impl Fn(&str) -> bool) -> Option<Self> {
        if !flagged("lod enabled") {
            return None;
        }
        let real = |name: &str| element.read_real(name).unwrap_or(0.0);
        let near = [real("LOD in distance"), real("LOD feather in delta")];
        let far = [real("LOD out distance"), real("LOD feather out delta")];
        Some(Self::postprocessed(near, far, flagged("lod always 1.0")))
    }

    /// `postprocess` (reach-tool `light_volume_definitions.cpp` l.175–195):
    /// an out distance of none is 20, an in distance at least epsilon, and
    /// each inverse over a feather of at least epsilon.
    pub(super) fn postprocessed(
        [near, feather_in]: [f32; 2],
        [far, feather_out]: [f32; 2],
        always_one: bool,
    ) -> Self {
        Self {
            near: near.max(EPSILON),
            feather_in,
            inverse_in: feather_in.max(EPSILON).recip(),
            far: if far.abs() < EPSILON {
                DEFAULT_LOD_OUT
            } else {
                far
            },
            feather_out,
            inverse_out: feather_out.max(EPSILON).recip(),
            always_one,
        }
    }
}

/// `c_light_volume_definition::calc_lod_amount` (the 360's 0x82EC77D0, MCC
/// sub_14029B0D0) at `distance` from the eye. `lod same in splitscreen`
/// only skips `x_distance_lod_scale`, which is 1 for one view \[inferred\].
pub(super) fn lod_amount(lod: Option<&ReachVolumeLod>, distance: f32) -> f32 {
    let Some(lod) = lod else {
        return 1.0;
    };
    // Sic: with `lod enabled`, `always 1.0` comes out 0, in both builds.
    if lod.always_one || distance <= lod.near || distance >= lod.far {
        return 0.0;
    }
    if distance < lod.near + lod.feather_in {
        (distance - lod.near) * lod.inverse_in
    } else if distance > lod.far - lod.feather_out {
        (lod.far - distance) * lod.inverse_out
    } else {
        1.0
    }
}

/// Reach's `max profile count` (tool 0x140DD0FA0): the most profiles the
/// two per-volume settings can make, 0 to 128, a NaN none.
pub(super) fn max_profile_count(
    density: &Gen3VolumeProperty,
    length: &Gen3VolumeProperty,
) -> usize {
    let most = (density.most() * length.most()).max(0.0);
    if !(MOST_PROFILES as f32 > most) {
        MOST_PROFILES as usize
    } else {
        most as i64 as usize
    }
}

/// `postprocess_build_precompiled_verts_if_needed` (tool 0x140DD2080, the
/// 360's 0x82EC6E58)'s test: unless `disable precompiled profiles`, a volume
/// that `force precompile profiles` or whose eight properties read nothing
/// but the profile's place bakes its rows, if it can make any.
pub(super) fn precompiles(
    flagged: impl Fn(&str) -> bool,
    properties: &[&Gen3VolumeProperty; 8],
    most: usize,
) -> bool {
    let profile_only = properties
        .iter()
        .all(|property| !property.reads_more_than_its_place());
    !flagged("disable precompiled profiles")
        && (flagged("force precompile profiles") || profile_only)
        && most != 0
}
