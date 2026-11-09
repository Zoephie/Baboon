//! A light volume evaluated for a frame: its profile layout, quads and draws.

use super::*;

/// What a volume's frame lays out before its profiles.
pub(in crate::app::model_preview) struct VolumeLayout {
    pub(in crate::app::model_preview) state: Gen3VolumeState,
    pub(in crate::app::model_preview) origin: [f32; 3],
    pub(in crate::app::model_preview) direction: [f32; 3],
    pub(in crate::app::model_preview) count: i32,
    /// `length × density`, whole or not.
    pub(in crate::app::model_preview) profiles: f32,
    /// `1 / density`.
    pub(in crate::app::model_preview) distance: f32,
    pub(in crate::app::model_preview) offset: f32,
    pub(in crate::app::model_preview) side_length: f32,
}

/// `c_light_volume_gpu::render`'s count, and the two sizes the shaders get.
pub(super) fn profile_layout(length: f32, density: f32) -> (i32, f32, f32) {
    if length <= 0.0 || density <= 0.0 {
        return (0, 0.0, 0.0);
    }
    let count = length * density;
    ((count as i32).min(MOST_PROFILES), count, 1.0 / density)
}

/// `light_volume_update`'s place along the row. A row of one sits at its start.
pub(in crate::app::model_preview) fn percentile(index: i32, profiles: f32) -> f32 {
    if profiles > 1.0 { index as f32 / (profiles - 1.0) } else { 0.0 }
}

/// What `light_volume.fx` divides alpha by to hold a volume's brightness at
/// any angle. Head on the spacing vanishes, and every profile stacks.
pub(super) fn overdraw(profiles: f32, profile_length: f32, spacing: f32) -> f32 {
    if spacing > 0.0 { profiles.min(profile_length / spacing) } else { profiles }.max(1e-6)
}

impl Gen3LightVolumeAt {
    /// The functions it reads off its object.
    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with time alone.
    pub(crate) fn animates(&self) -> bool {
        let volume = &*self.definition;
        let reads_time = [
            &volume.length, &volume.offset, &volume.density, &volume.profile_length,
            &volume.thickness, &volume.color, &volume.alpha, &volume.intensity,
        ];
        self.times.moves()
            || reads_time.iter().any(|property| property.inputs().any(|input| input == Gen3VolumeInput::GameTime))
    }

    /// Its sort position: `submit` hands the transparents its location.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        posed_on(self.node, self.position, self.forward, bones).0
    }

    pub(crate) fn sampler(&self) -> Option<Sampler> {
        self.definition.sampler
    }

    pub(crate) fn bitmap(&self) -> Option<&Gen3Bitmap> {
        self.definition.bitmap.as_ref()
    }

    /// Reach's `circular` albedo's `center_offset` and `falloff`.
    pub(crate) fn circular(&self) -> Option<[f32; 2]> {
        self.definition.circular
    }

    /// Reach's `depth_fade_range`, when it fades against the scene.
    pub(crate) fn depth_fade(&self) -> Option<f32> {
        self.definition.depth_fade
    }

    pub(crate) fn sort_layer(&self) -> i32 {
        self.definition.sort_layer
    }

    pub(crate) fn blend(&self) -> Gen3Blend {
        self.definition.blend
    }

    /// The states this frame, or none while its event is not running.
    fn state(&self, values: Option<&FunctionValues>, time: f32, onset: f32) -> Option<Gen3VolumeState> {
        let read = |name: &str, none: f32| {
            if name.is_empty() {
                return none;
            }
            values.and_then(|values| values.get(name)).copied().unwrap_or(0.0)
        };
        // `attachments_update` runs the effect while its function is on.
        let effect_a = read(&self.scale_by, 1.0);
        if effect_a <= 0.0 {
            return None;
        }
        Some(Gen3VolumeState {
            percentile: 0.0,
            game_time: time.rem_euclid(1.0),
            age: self.times.age(self.event, time - onset)?,
            seeds: self.seeds,
            lod: 0.0,
            effect_a,
            effect_b: read(&self.secondary_by, 1.0),
        })
    }

    /// What `c_light_volume_gpu::render` works out once a frame, or none
    /// while it draws nothing.
    pub(in crate::app::model_preview) fn layout(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Option<VolumeLayout> {
        let mut state = self.state(values, time, onset)?;
        let volume = &*self.definition;
        let (origin, turned) = posed_on(self.node, self.position, self.forward, bones);
        if volume.reach {
            // `c_light_volume_gpu::render` (Reach: the 360's 0x826B80B8, MCC
            // sub_140887B70; Halo 4: MCC 0x1408ECFD0): by the eye's whole
            // distance from the location, nothing at no amount, and the
            // amount is state 6.
            let from_eye = std::array::from_fn(|axis| origin[axis] - view.eye[axis]);
            let amount = lod_amount(volume.lod.as_ref(), length(from_eye));
            if amount <= 0.0 {
                return None;
            }
            state.lod = amount;
        }
        let length = volume.length.scalar(&state);
        let density = volume.density.scalar(&state);
        let (count, profiles, distance) = profile_layout(length, density);
        let direction = if self.world_oriented { self.forward } else { turned };
        Some(VolumeLayout {
            state,
            origin,
            direction: clamped_unit(direction),
            count,
            profiles,
            distance,
            offset: volume.offset.scalar(&state),
            side_length: volume.profile_length.scalar(&state),
        })
    }

    /// Halo 4's material, when it is Halo 4's.
    pub(crate) fn h4_material(&self) -> Option<&H4EffectMaterial> {
        self.definition.h4.as_ref().map(|(material, _)| material)
    }

    /// Each profile's quad this frame, nearest the marker first, at the area
    /// screen effects' exposure `stops`.
    pub(crate) fn quads(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<AntennaQuad> {
        let Some(VolumeLayout { mut state, origin, direction, count, profiles, distance, offset, side_length }) =
            self.layout(values, bones, view, time, onset)
        else {
            return Vec::new();
        };
        let volume = &*self.definition;
        // A baked volume draws `max profile count` rows, spaced and overdrawn
        // by the live settings; that it draws none where they make none was
        // not traced.
        let baked = volume.precompiled.as_deref().filter(|_| count > 0);
        let count = baked.map_or(count, |rows| rows.len() as i32);
        let mut quads = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            state.percentile = percentile(index, profiles);
            let along = offset + distance * index as f32;
            let place: [f32; 3] = std::array::from_fn(|axis| origin[axis] + direction[axis] * along);
            let row = baked.map(|rows| rows[index as usize]);
            let thickness = row.map_or_else(|| volume.thickness.scalar(&state), |(_, thickness)| thickness);
            let to_profile = clamped_unit(std::array::from_fn(|axis| place[axis] - view.eye[axis]));
            let sine = length(cross(direction, to_profile)).min(1.0);
            let drawn_length = thickness + (side_length - thickness) * sine;
            let up = clamped_unit(cross(to_profile, direction));
            let across = cross(to_profile, up);
            let corner = |u: f32, v: f32| {
                let (x, y) = ((u - 0.5) * drawn_length, (v - 0.5) * thickness);
                let [a, b, c] = std::array::from_fn(|axis| place[axis] + across[axis] * x + up[axis] * y);
                [a, b, c, u, v]
            };
            let ratio = volume.brightness_ratio + (1.0 - volume.brightness_ratio) * sine;
            let overdrawn = overdraw(profiles, drawn_length, distance * sine);
            quads.push(AntennaQuad {
                corners: [corner(0.0, 0.0), corner(1.0, 0.0), corner(0.0, 1.0), corner(1.0, 1.0)],
                color: volume.profile_color(&state, row.map(|(color, _)| color), [ratio, overdrawn], stops),
            });
        }
        quads
    }
}

attached!(Gen3LightVolumeAt);
