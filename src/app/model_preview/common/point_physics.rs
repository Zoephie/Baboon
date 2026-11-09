//! A `point_physics` tag as its postprocess leaves it: what CE's particles
//! and flags, and every game's antennas, move by.

use blam_tags::TagFile;

use super::fields::has_flag;

/// `physics_constants_reset`: gravity, and air in a level without water.
pub(in crate::app::model_preview) const GRAVITY: f32 = 4.171_259_4;

pub(in crate::app::model_preview) const AIR_MASS_OVER_RADIUS_CUBED: f32 = 0.0011 * MASS_OF_DENSITY;

/// What `point_physics` postprocess (`sub_525660`) makes of a density.
pub(in crate::app::model_preview) const MASS_OF_DENSITY: f32 = 0.001 * 4.188_790_3 * 28_316_846.0;

/// A `point_physics`, as its postprocess leaves it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PointPhysics {
    pub(in crate::app::model_preview) mass_over_radius_cubed: f32,
    pub(in crate::app::model_preview) air_buoyancy: f32,
    pub(in crate::app::model_preview) air_friction: f32,
    pub(in crate::app::model_preview) surface_friction: f32,
    pub(in crate::app::model_preview) elasticity: f32,
    pub(in crate::app::model_preview) collides_with_structure: bool,
}

impl PointPhysics {
    pub(in crate::app::model_preview) fn read(tag: &TagFile) -> Self {
        let root = tag.root();
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let set = |option: &str| has_flag(root, "flags", option);
        Self::new(
            real("density"),
            set("no gravity"),
            set("collides with structures"),
            real("air friction"),
            real("surface friction"),
            real("elasticity"),
        )
    }

    /// What the postprocess makes of a tag's `density`, its `flags`' `no
    /// gravity` and `collides with structures`, `air friction`, `surface
    /// friction` and `elasticity`.
    pub(in crate::app::model_preview) fn new(
        density: f32,
        no_gravity: bool,
        collides_with_structure: bool,
        air_friction: f32,
        surface_friction: f32,
        elasticity: f32,
    ) -> Self {
        Self {
            mass_over_radius_cubed: density * MASS_OF_DENSITY,
            air_buoyancy: if no_gravity {
                0.0
            } else {
                1.0 - 2.0 * density / (density + 0.0011)
            },
            air_friction: air_friction * 10_000.0,
            surface_friction,
            elasticity,
            collides_with_structure,
        }
    }

    pub(in crate::app::model_preview) fn mass(&self, radius: f32) -> f32 {
        self.mass_over_radius_cubed * radius * radius * radius
    }

    /// `point_physics_definition_interpolate`: the runtime values from one to
    /// the other by `t`; the flags stay the first's.
    pub(in crate::app::model_preview) fn lerp(&self, other: &Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a * (1.0 - t) + b * t;
        Self {
            mass_over_radius_cubed: mix(self.mass_over_radius_cubed, other.mass_over_radius_cubed),
            air_buoyancy: mix(self.air_buoyancy, other.air_buoyancy),
            air_friction: mix(self.air_friction, other.air_friction),
            surface_friction: mix(self.surface_friction, other.surface_friction),
            elasticity: mix(self.elasticity, other.elasticity),
            collides_with_structure: self.collides_with_structure,
        }
    }

    /// The air friction tool leaves, scaled by 10000.
    pub(in crate::app::model_preview) fn air_friction(&self) -> f32 {
        self.air_friction
    }

    /// `point_physics_update` with a force on the point, which first moves its
    /// velocity by the force over the mass of it and the air it displaces.
    pub(in crate::app::model_preview) fn step_with_force(
        &self,
        place: &mut [f32; 3],
        velocity: &mut [f32; 3],
        force: [f32; 3],
        radius: f32,
        ground: f32,
        dt: f32,
    ) -> bool {
        if dt == 0.0 {
            return false;
        }
        let bulk =
            (AIR_MASS_OVER_RADIUS_CUBED + self.mass_over_radius_cubed) * radius * radius * radius;
        if bulk != 0.0 {
            for (velocity, force) in velocity.iter_mut().zip(force) {
                *velocity += force * (dt / bulk);
            }
        }
        self.step(place, velocity, radius, ground, dt)
    }

    /// `point_physics_update` in still air over a flat floor at `ground`: true
    /// where it touched it, the only structure a preview has.
    pub(in crate::app::model_preview) fn step(
        &self,
        place: &mut [f32; 3],
        velocity: &mut [f32; 3],
        radius: f32,
        ground: f32,
        dt: f32,
    ) -> bool {
        let bulk =
            (AIR_MASS_OVER_RADIUS_CUBED + self.mass_over_radius_cubed) * radius * radius * radius;
        let drag = self.air_friction * radius * radius;
        velocity[2] += GRAVITY * self.air_buoyancy * dt;
        let slowing = if bulk != 0.0 {
            (dt / bulk * drag).clamp(0.0, 1.0)
        } else if drag != 0.0 {
            1.0
        } else {
            0.0
        };
        for axis in 0..3 {
            velocity[axis] -= velocity[axis] * slowing;
        }
        let mut left = dt;
        let mut touched = false;
        for _ in 0..3 {
            let next: [f32; 3] = std::array::from_fn(|axis| place[axis] + velocity[axis] * left);
            if !self.collides_with_structure || next[2] >= ground || velocity[2] >= 0.0 {
                *place = next;
                return touched;
            }
            touched = true;
            let t = ((place[2] - ground) / (place[2] - next[2])).clamp(0.0, 1.0);
            let hit: [f32; 3] =
                std::array::from_fn(|axis| place[axis] + (next[axis] - place[axis]) * t);
            // `component_vectors_from_normal3d`: along the floor, and into it.
            let into = velocity[2];
            velocity[0] *= 1.0 - self.surface_friction;
            velocity[1] *= 1.0 - self.surface_friction;
            velocity[2] = -into * self.elasticity;
            *place = [hit[0], hit[1], hit[2] + radius.min(0.005)];
            left -= t * left;
            if left == 0.0 {
                break;
            }
        }
        touched
    }
}
