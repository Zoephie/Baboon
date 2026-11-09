//! A strip's geometry: its basis, profiles and vertices, and the beam and contrail vertices for a frame.

use super::*;

/// `cross_section_world_basis`. Reach's has no guard for straight up, where
/// `normalize(0)` takes every corner to NaN and the strip does not draw; the
/// preview keeps Halo 3's basis for it.
pub(super) fn world_basis(direction: [f32; 3]) -> [[f32; 3]; 2] {
    let product = cross(direction, [0.0, 0.0, 1.0]);
    if product == [0.0; 3] {
        return [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
    }
    let first = clamped_unit(product);
    [first, clamped_unit(cross(first, direction))]
}

/// One profile, as both systems' vertex shaders read it.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Profile {
    pub(super) place: [f32; 3],
    pub(super) direction: [f32; 3],
    pub(super) offset: [f32; 2],
    pub(super) size: f32,
    pub(super) rotation: f32,
    /// Lengthwise in world units, for the texture.
    pub(super) length: f32,
    pub(super) color: [f32; 4],
    pub(super) palette: f32,
    pub(super) black_point: f32,
}

/// `default_vs`, for a run of profiles.
fn strip_vertices(
    look: &StripLook,
    profiles: &[Profile],
    view: &ViewFrame,
    time: f32,
    uv_offset: [f32; 2],
    origin_fade: impl Fn(&Profile, [f32; 3]) -> f32,
    edge_view: impl Fn([f32; 3]) -> [f32; 3],
) -> Vec<ParticleCorner> {
    let shape = look.shape;
    let vertex = |profile: &Profile, strip: usize, offset: usize| {
        let world = world_basis(profile.direction);
        let basis = if shape == StripShape::Ribbon {
            let first = clamped_unit(cross(minus(profile.place, view.eye), profile.direction));
            [first, clamped_unit(cross(first, profile.direction))]
        } else {
            world
        };
        let (sin, cos) = (std::f32::consts::TAU * profile.rotation).sin_cos();
        let basis: [[f32; 3]; 2] = [
            std::array::from_fn(|axis| cos * basis[0][axis] + sin * basis[1][axis]),
            std::array::from_fn(|axis| -sin * basis[0][axis] + cos * basis[1][axis]),
        ];
        let [x, y] = shape.across(strip, offset).map(|side| side * profile.size);
        let place: [f32; 3] = std::array::from_fn(|axis| {
            profile.place[axis]
                + basis[0][axis] * x
                + basis[1][axis] * y
                + world[0][axis] * profile.offset[0]
                + world[1][axis] * profile.offset[1]
        });
        let shifted = [profile.length, shape.v_shift(strip, offset)];
        let uv: [f32; 2] = std::array::from_fn(|axis| {
            shifted[axis] * look.tiling[axis]
                + [0.0, 0.5][axis]
                + time * look.scroll[axis]
                + uv_offset[axis]
        });
        let mut color = profile.color;
        color[3] *= origin_fade(profile, place);
        if let Some([range, cutoff]) = look.edge_fade {
            let normal = clamped_unit(cross(basis[0], basis[1]));
            let angle = dot(edge_view(place), normal).abs().min(1.0).acos();
            color[3] *= look.fade(range * (angle - cutoff));
        }
        if let Some([range, cutoff]) = look.angle_fade {
            color[3] *= angle_fade(place, profile, view.eye, range, cutoff);
        }
        ParticleCorner {
            place,
            sprite: uv,
            next_sprite: uv,
            billboard: uv,
            color,
            extra: [0.0, profile.palette, profile.black_point],
            lean: [[0.0; 3]; 2],
            normal: [0.0, 0.0, 1.0],
        }
    };
    let mut vertices = Vec::with_capacity(profiles.len() * shape.strips() * 6);
    for strip in 0..shape.strips() {
        for pair in profiles.windows(2) {
            // The n-gon's sides wind so the inward ones cull.
            if let (StripShape::Ngon(_), false) = (shape, look.double_sided) {
                let middle: [f32; 3] = {
                    let side = vertex(&pair[0], strip, 0).place;
                    let other = vertex(&pair[0], strip, 1).place;
                    std::array::from_fn(|axis| (side[axis] + other[axis]) * 0.5)
                };
                if dot(minus(middle, pair[0].place), minus(view.eye, middle)) < 0.0 {
                    continue;
                }
            }
            let quad = [
                vertex(&pair[0], strip, 0),
                vertex(&pair[0], strip, 1),
                vertex(&pair[1], strip, 1),
                vertex(&pair[1], strip, 0),
            ];
            vertices.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
        }
    }
    vertices
}

/// Reach's beam angle fade (`effects__beam.hlsl` l.296–305), at a corner
/// `place` of `profile`: by how far from face on the corner's side is seen,
/// across the beam's direction. Angle 0 faces the camera.
pub(super) fn angle_fade(place: [f32; 3], profile: &Profile, eye: [f32; 3], range: f32, cutoff: f32) -> f32 {
    let profile_normal = clamped_unit(minus(place, profile.place));
    let camera_to_vertex = clamped_unit(minus(place, eye));
    let profile_ortho = clamped_unit(cross(camera_to_vertex, profile.direction));
    let profile_camera_ortho = clamped_unit(cross(profile_ortho, profile.direction));
    let angle = dot(profile_camera_ortho, profile_normal).abs().min(1.0).acos();
    // A range of 0 divides by nothing, as the game does: a step at the begin
    // angle, and a NaN on it.
    saturate(1.0 - (angle - cutoff) / range)
}

/// A profile's colour as `default_vs` exposes it, by `particle_exposure`'s
/// rule, at the area screen effects' `stops`.
pub(super) fn exposed(look: &StripLook, color: [f32; 3], alpha: f32, intensity: f32, stops: f32) -> [f32; 4] {
    let light = particle_exposure(look.shader.blend, true, look.reach, stops);
    let [red, green, blue] = color.map(|channel| channel.clamp(0.0, 1.0) * intensity * light);
    [red, green, blue, alpha.clamp(0.0, 1.0) * intensity]
}

/// `pulse`'s split of a beam: how far its `count` profiles reach.
pub(super) fn beam_profiles(capped: f32, density: f32) -> usize {
    (capped * density).ceil().clamp(FEWEST_PROFILES, MOST_PROFILES as f32) as usize
}

impl H3StripAt {
    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }


    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn shader(&self) -> &H3SpriteShader {
        &self.look.shader
    }

    /// How much nearer than its location it sorts: `add_element` takes
    /// `get_sort_bias` off its view depth.
    pub(crate) fn sort_bias(&self) -> f32 {
        self.look.sort_bias
    }

    /// Its sort position, as `submit` hands it over.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        posed_on(self.node, self.position, self.forward, bones).0
    }

    /// The states this frame, or none while its event is not running.
    pub(super) fn state(&self, values: Option<&FunctionValues>, time: f32, onset: f32) -> Option<StripState> {
        let read = |name: &str, none: f32| {
            if name.is_empty() {
                return none;
            }
            values.and_then(|values| values.get(name)).copied().unwrap_or(0.0)
        };
        let effect_a = read(&self.scale_by, 1.0);
        if effect_a <= 0.0 {
            return None;
        }
        Some(StripState {
            game_time: time.rem_euclid(1.0),
            age: self.times.age(self.event, time - onset)?,
            seeds: self.seeds,
            effect_a,
            effect_b: read(&self.secondary_by, 1.0),
            ..Default::default()
        })
    }

    pub(super) fn beam_vertices(
        &self,
        beam: &BeamDefinition,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<ParticleCorner> {
        let Some(mut state) = self.state(values, time, onset) else {
            return Vec::new();
        };
        let (origin, turned) = posed_on(self.node, self.position, self.forward, bones);
        let forward = if self.world_oriented { self.forward } else { turned };
        let uncapped = beam.length.scalar(&state);
        let reach = self.end.map(|(node, end)| minus(posed_on(node, end, [1.0, 0.0, 0.0], bones).0, origin));
        let (direction, cap) = match reach {
            Some(reach) => (clamped_unit(reach), dot(reach, reach).sqrt()),
            None => (clamped_unit(forward), 0.0),
        };
        let capped = if cap != 0.0 && cap <= uncapped { cap } else { uncapped };
        state.length = capped;
        let density = beam.density.scalar(&state);
        let count = beam_profiles(capped, density);
        let step = (density * capped).max(1e-6).recip();
        let cap_percentage = if uncapped != 0.0 { capped / uncapped } else { 1.0 };
        let offset = beam.offset.scalar(&state);
        let look = &*self.look;
        let profiles: Vec<Profile> = (0..count)
            .map(|index| {
                let percentile = if index == count - 1 { 1.0 } else { index as f32 * step };
                state.position = percentile;
                state.uncapped_position = percentile * cap_percentage;
                let along = offset + capped * percentile;
                let alpha = beam.alpha.scalar(&state);
                let intensity = beam.intensity.scalar(&state);
                Profile {
                    place: std::array::from_fn(|axis| origin[axis] + direction[axis] * along),
                    direction,
                    size: beam.thickness.scalar(&state),
                    rotation: beam.rotation.scalar(&state),
                    length: (percentile * capped).abs(),
                    color: exposed(look, beam.color.color(&state), alpha, intensity, stops),
                    palette: beam.palette.scalar(&state).clamp(0.0, 0.9999),
                    black_point: beam.black_point.scalar(&state).clamp(0.0, 0.9999),
                    ..Default::default()
                }
            })
            .collect();
        let fade = look.origin_fade;
        let scroll_time = look.scroll_time(REACH_BEAM_SCROLL_PERIOD, time);
        strip_vertices(
            look,
            &profiles,
            view,
            scroll_time,
            [0.0; 2],
            |_, place| {
                fade.map_or(1.0, |[range, cutoff]| {
                    let from = minus(place, origin);
                    look.fade(range * (dot(from, from).sqrt() - cutoff))
                })
            },
            |place| clamped_unit(minus(place, view.eye)),
        )
    }

    pub(super) fn contrail_vertices(
        &self,
        contrail: &ContrailDefinition,
        running: &RunningContrail,
        values: Option<&FunctionValues>,
        view: &ViewFrame,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<ParticleCorner> {
        if running.profiles.len() < 2 {
            return Vec::new();
        }
        let base = StripState {
            speed: running.speed,
            ..self.state(values, time, onset).unwrap_or(StripState { game_time: time.rem_euclid(1.0), age: 1.0, seeds: self.seeds, ..Default::default() })
        };
        let look = &*self.look;
        let initial = if contrail.tint_from_lightmap { lightmap_tint(lighting) } else { [1.0; 3] };
        let live = &running.profiles;
        let profiles: Vec<Profile> = live
            .iter()
            .enumerate()
            .map(|(index, profile)| {
                let state = StripState { profile_age: profile.age, profile_seeds: profile.seeds, ..base };
                let direction = match live.get(index + 1) {
                    Some(next) => minus(next.place, profile.place),
                    None => minus(profile.place, live[index - 1].place),
                };
                let alpha = contrail.alpha.scalar(&state).clamp(0.0, 1.0)
                    * contrail.secondary_alpha.scalar(&state).clamp(0.0, 1.0)
                    * profile.alpha;
                let [x, y, _] = contrail.offset.vector(&state);
                let intensity = contrail.intensity.scalar(&state);
                let mut color = exposed(look, contrail.color.color(&state), alpha, intensity, stops);
                (0..3).for_each(|channel| color[channel] *= initial[channel]);
                Profile {
                    place: profile.place,
                    direction,
                    offset: [x, y],
                    size: contrail.size.scalar(&state),
                    rotation: contrail.rotation.scalar(&state).rem_euclid(1.0),
                    length: profile.length,
                    color,
                    palette: contrail.palette.scalar(&state).rem_euclid(1.0),
                    black_point: contrail.black_point.scalar(&state).rem_euclid(1.0),
                }
            })
            .collect();
        let fade = look.origin_fade;
        let uv_offset = [
            if contrail.random_u { self.seeds[0] } else { 0.0 },
            if contrail.random_v { self.seeds[1] } else { 0.0 },
        ];
        let scroll_time = look.scroll_time(REACH_CONTRAIL_SCROLL_PERIOD, time);
        strip_vertices(
            look,
            &profiles,
            view,
            scroll_time,
            uv_offset,
            |profile, _| fade.map_or(1.0, |[range, cutoff]| look.fade(range * (profile.length - cutoff))),
            |_| view.forward,
        )
    }
}
