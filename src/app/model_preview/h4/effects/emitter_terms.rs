//! What Halo 4's particle update adds to each emitter over Reach's.

use blam_tags::TagStruct;

use super::super::super::particles::{
    EmitterTerms, EmitterTermsRead, ParticleProperty, ParticleState,
};

/// What Halo 4's `particle_update.fx` adds over Reach's: its system's
/// `particles live forever` and `disable velocity`, its `size scale`
/// (`m_scaleMultiplier`), and the emitter's scale x and y and alpha white
/// point.
#[derive(Debug)]
pub(in crate::app::model_preview) struct H4EmitterTerms {
    live_forever: bool,
    disable_velocity: bool,
    size_scale: f32,
    scale: Option<[ParticleProperty; 2]>,
    white_point: Option<ParticleProperty>,
}

/// An emitter's Halo 4 terms, over what the shared reader has read.
pub(in crate::app::model_preview) fn read_h4_emitter_terms(
    emitter: TagStruct<'_>,
    read: EmitterTermsRead,
) -> Box<dyn EmitterTerms> {
    // Read past Reach's allocation [inferred: their seeds follow].
    let property = |name: &str| {
        emitter
            .descend(name)
            .and_then(|holder| ParticleProperty::unallocated(holder))
    };
    let scale = property("particle scale x")
        .zip(property("particle scale y"))
        .map(|(x, y)| [x, y]);
    let size_scale = read.size_scale;
    Box::new(H4EmitterTerms {
        live_forever: read.live_forever,
        disable_velocity: read.disable_velocity,
        // `c_particle_system_definition::postprocess_block_proc`.
        size_scale: if size_scale < 1e-4 { 1.0 } else { size_scale },
        scale,
        white_point: property("particle alpha white point"),
    })
}

impl EmitterTerms for H4EmitterTerms {
    /// `liveForever`: the age comes round, and from just short of 1 back to 0.
    fn age(&self, age: f32) -> f32 {
        if !self.live_forever {
            return age;
        }
        let age = age.fract();
        if age > 0.99 { 0.0 } else { age }
    }

    fn travel(&self, dt: f32) -> f32 {
        if self.disable_velocity {
            0.0
        } else {
            dt * self.size_scale
        }
    }

    /// `m_size`'s and `m_aspect`'s factors: scale y and the size scale, and
    /// scale x over scale y.
    fn size_and_aspect(&self, state: &ParticleState) -> [f32; 2] {
        let [x, y] = self
            .scale
            .as_ref()
            .map_or([1.0; 2], |[x, y]| [x.scalar(state), y.scalar(state)]);
        let aspect = if y != 0.0 { x / y } else { 1.0 };
        [y * self.size_scale, aspect]
    }

    fn white_point(&self) -> Option<&ParticleProperty> {
        self.white_point.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terms of their own, with no scale or white point.
    fn h4_terms(live_forever: bool, disable_velocity: bool, size_scale: f32) -> H4EmitterTerms {
        H4EmitterTerms {
            live_forever,
            disable_velocity,
            size_scale,
            scale: None,
            white_point: None,
        }
    }

    /// `liveForever` brings the age round, and from past 0.99 back to 0;
    /// without it the age runs on to death.
    #[test]
    fn a_particle_that_lives_forever_comes_round() {
        let forever = h4_terms(true, false, 1.0);
        assert!((forever.age(1.25) - 0.25).abs() < 1e-6);
        assert_eq!(forever.age(0.995), 0.0);
        assert_eq!(forever.age(0.5), 0.5);
        assert_eq!(h4_terms(false, false, 1.0).age(1.25), 1.25);
    }

    /// `disableVelocity` holds it in place; otherwise the size scale
    /// multiplies the step.
    #[test]
    fn velocity_moves_by_the_size_scale_unless_disabled() {
        assert_eq!(h4_terms(false, true, 2.0).travel(0.5), 0.0);
        assert_eq!(h4_terms(false, false, 2.0).travel(0.5), 1.0);
    }

    /// Scale y and the size scale go into the size, scale x over scale y
    /// into the aspect; with neither the size scale alone.
    #[test]
    fn scale_x_and_y_bake_into_size_and_aspect() {
        let base = ParticleState::default();
        let none = h4_terms(false, false, 3.0);
        assert_eq!(none.size_and_aspect(&base), [3.0, 1.0]);
        let constant = |value: f32| ParticleProperty::fixed(value).unwrap();
        let scaled = H4EmitterTerms {
            scale: Some([constant(2.0), constant(0.5)]),
            ..h4_terms(false, false, 1.0)
        };
        assert_eq!(scaled.size_and_aspect(&base), [0.5, 4.0]);
    }
}
