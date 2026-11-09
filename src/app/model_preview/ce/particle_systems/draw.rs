//! A particle system's sprite quads.

use super::read::{COLOR, CePctlDef, SCALE};
use super::sim::{CeParticleState, CeRunningSystem};
use super::{CeRandom, CeSecondaryDraw, FramebufferBlend, ViewFrame, dot, unit_or_unchanged};

/// `particle_system_render` for one system.
#[allow(clippy::too_many_arguments)]
pub(super) fn system_quads(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    index: usize,
    view: &ViewFrame,
    pixel: &impl Fn([f32; 3]) -> f32,
    random: &mut CeRandom,
    time: f32,
    quads: &mut Vec<CePctlQuad>,
) {
    let lighting = running.lighting;
    for (kind, run) in def.types.iter().zip(&mut running.types) {
        if run.state.is_none() || kind.flags.disabled {
            continue;
        }
        let multipliers = run.variables;
        for particle in &mut run.particles {
            if !particle.valid {
                continue;
            }
            let Some(current) = particle.state.and_then(|at| kind.particle_states.get(at)) else {
                continue;
            };
            let next = particle
                .transition
                .and_then(|at| kind.particle_states.get(at));
            let mut fade = 1.0;
            if next.is_some() {
                fade = if particle.length != 0.0 {
                    (particle.time_left / particle.length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }
            let mix = |at: usize| match next {
                Some(_) => {
                    particle.transition_variables[at] * (1.0 - fade) + particle.variables[at] * fade
                }
                None => particle.variables[at],
            };
            let mut color: [f32; 4] =
                std::array::from_fn(|channel| mix(3 + channel) * multipliers[COLOR + channel]);
            // Alpha blend takes the light where the system is.
            if current.blend == FramebufferBlend::AlphaBlend {
                for channel in 0..3 {
                    color[1 + channel] *= lighting[channel];
                }
            }
            let scale = mix(0) * multipliers[SCALE];
            let mut fade_next = 0.0;
            match next {
                Some(next)
                    if next.blend != current.blend
                        || next.map_flags != current.map_flags
                        || next.sequence != current.sequence =>
                {
                    fade_next = 1.0 - fade
                }
                _ => fade = 1.0,
            }
            let sequence = current.sequence + i32::from(kind.rotational);
            let count = current.sheet.count(sequence);
            if count == 0 {
                continue;
            }
            let sprite = if particle.sprite == -1.0 {
                let at = random.index(count);
                particle.sprite = at as f32;
                at
            } else {
                (particle.sprite as i32).rem_euclid(count as i32) as usize
            };
            let size = if scale == 0.0 {
                pixel(particle.position)
            } else {
                scale
            };
            for (state, fade, nudge) in [
                (current, fade, 0.0),
                (next.unwrap_or(current), fade_next, 0.001),
            ] {
                if fade <= 0.01 {
                    continue;
                }
                // The second sits a thousandth nearer, in view space.
                let center: [f32; 3] = std::array::from_fn(|axis| {
                    particle.position[axis] - view.forward[axis] * nudge
                });
                let group = quads.len();
                let shown = Shown {
                    state,
                    time,
                    radius: current.radius,
                    center,
                    axis: particle.axis,
                    rotation: particle.rotation,
                    size,
                    color,
                    fade,
                    group,
                };
                if kind.rotational {
                    shown.rotational(sprite, kind.flags.rotational_sideways, view, index, quads);
                } else {
                    shown.simple(sprite, view, index, quads);
                }
            }
        }
    }
}

/// A sprite `particle_system_render` builds.
#[derive(Debug, Clone)]
pub(crate) struct CePctlQuad {
    pub(crate) system: usize,
    /// Shared by the quads of one draw of one particle state, which
    /// `particle_system_render` sends as a batch of its own.
    pub(crate) group: usize,
    pub(crate) sheet: usize,
    pub(crate) image: usize,
    pub(crate) blend: FramebufferBlend,
    pub(crate) nonlinear_tint: bool,
    pub(crate) sort_bias: bool,
    pub(crate) secondary: Option<CeSecondaryDraw>,
    pub(crate) color: [f32; 4],
    pub(crate) corners: [[f32; 5]; 4],
}

/// What one draw of a particle takes.
struct Shown<'a> {
    state: &'a CeParticleState,
    /// The game's time, for the second map's animation.
    time: f32,
    /// The current state's radius multiplier.
    radius: f32,
    center: [f32; 3],
    axis: [f32; 3],
    rotation: f32,
    size: f32,
    /// Alpha, red, green, blue.
    color: [f32; 4],
    fade: f32,
    /// What its quads carry as [`CePctlQuad::group`].
    group: usize,
}

impl Shown<'_> {
    /// `build_sprite`'s color: the fade is the alpha unless the blend is alpha
    /// blend or the tint nonlinear.
    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        sequence: i32,
        sprite: usize,
        rotation: f32,
        fade: f32,
        mirror: bool,
        view: &ViewFrame,
        system: usize,
        quads: &mut Vec<CePctlQuad>,
    ) {
        let Some((image, corners)) = self.state.sheet.corners(
            sequence,
            sprite,
            self.center,
            rotation,
            self.size,
            mirror,
            view,
        ) else {
            return;
        };
        let [alpha, red, green, blue] = self.color;
        let alpha =
            if self.state.blend != FramebufferBlend::AlphaBlend && !self.state.nonlinear_tint {
                fade
            } else {
                alpha * fade
            };
        // `particle_system_render` gives the shader the first state's radius
        // multiplier for its second map's radius, for either draw.
        let secondary = self
            .state
            .secondary
            .as_ref()
            .map(|map| map.draw(self.time, self.radius));
        quads.push(CePctlQuad {
            system,
            group: self.group,
            sheet: self.state.sheet_index,
            image,
            blend: self.state.blend,
            nonlinear_tint: self.state.nonlinear_tint,
            sort_bias: self.state.sort_bias,
            secondary,
            color: [red, green, blue, alpha.clamp(0.0, 1.0)],
            corners,
        });
    }

    fn simple(&self, sprite: usize, view: &ViewFrame, system: usize, quads: &mut Vec<CePctlQuad>) {
        self.push(
            self.state.sequence,
            sprite,
            self.rotation,
            self.fade,
            false,
            view,
            system,
            quads,
        );
    }

    /// `build_sprite_rotational`: end on, the next sequence's sprite; side
    /// on, the sequence's sprite for the angle it turns through, laid along
    /// the axis on screen; between, both.
    fn rotational(
        &self,
        sprite: usize,
        sideways: bool,
        view: &ViewFrame,
        system: usize,
        quads: &mut Vec<CePctlQuad>,
    ) {
        let sheet = &self.state.sheet;
        let first = self.state.sequence;
        let toward = unit_or_unchanged(std::array::from_fn(|axis| {
            self.center[axis] - view.eye[axis]
        }));
        let axis = unit_or_unchanged(self.axis);
        let angle = dot(toward, axis).clamp(-1.0, 1.0).acos();
        let off = angle - std::f32::consts::FRAC_PI_2;
        let end_on = (off * off * 0.405_284_7).clamp(0.0, 1.0);
        let turns = self.rotation * 0.159_154_94;
        // `fmod` keeps the sign, and `build_sprite` draws no sprite below 0.
        if end_on > 0.05 {
            let (sprite, rotation, mirror) = if sideways {
                let count = sheet.count(first + 1) as f32;
                let at = ((count * turns + 0.5) % count + sprite as f32) as i32;
                (
                    if off < 0.0 {
                        count as i32 - sprite as i32
                    } else {
                        at
                    },
                    0.0,
                    false,
                )
            } else {
                (sprite as i32, self.rotation, off < 0.0)
            };
            if let Ok(sprite) = usize::try_from(sprite) {
                self.push(
                    first + 1,
                    sprite,
                    rotation,
                    end_on * self.fade,
                    mirror,
                    view,
                    system,
                    quads,
                );
            }
        }
        if 1.0 - end_on > 0.05 {
            let count = sheet.count(first) as f32;
            let at = ((count * turns + 0.5) % count) as i32;
            let screen = [dot(axis, view.right), dot(axis, view.up)];
            let rotation = screen[1].atan2(screen[0]);
            if let Ok(at) = usize::try_from(at) {
                self.push(
                    first,
                    at,
                    rotation,
                    (1.0 - end_on) * self.fade,
                    false,
                    view,
                    system,
                    quads,
                );
            }
        }
    }
}

/// `particle_system_render` for a system on no object.
pub(crate) fn unattached_quads(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    index: usize,
    view: &ViewFrame,
    pixel: impl Fn([f32; 3]) -> f32,
    random: &mut CeRandom,
    time: f32,
) -> Vec<CePctlQuad> {
    let mut quads = Vec::new();
    system_quads(def, running, index, view, &pixel, random, time, &mut quads);
    quads
}
