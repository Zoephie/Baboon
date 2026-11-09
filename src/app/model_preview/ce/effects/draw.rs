//! An effect particle's sprite quad and its basis.

use super::sim::rotate_about;
use super::{CeSecondaryDraw, FramebufferBlend, ViewFrame, cross, dot, unit_or_unchanged};

/// What a sprite draws: the effect and particle it came from, its corners
/// with texture coordinates, and the vertex color the `effect` shader reads.
#[derive(Debug, Clone)]
pub(crate) struct CeParticleQuad {
    /// [`def_key`] of the effect it came from, and which of its particles.
    pub(crate) effect: usize,
    pub(crate) def: usize,
    pub(crate) blend: FramebufferBlend,
    pub(crate) nonlinear_tint: bool,
    pub(crate) sort_bias: bool,
    pub(crate) secondary: Option<CeSecondaryDraw>,
    pub(crate) image: usize,
    pub(crate) corners: [[f32; 5]; 4],
    pub(crate) color: [f32; 4],
}

/// `build_sprite_compute_basis`: the sprite's across (its u) and up (its v).
/// Screen facing takes the view's; parallel to its direction runs across
/// along it, up square to it and the way to the eye; perpendicular lies
/// across it, square to world up, or to world left (which the game calls
/// `viewer_space_world_forward`) where it is near vertical, turned a quarter
/// about it.
pub(super) fn sprite_basis(
    orientation: i32,
    direction: [f32; 3],
    to_particle: [f32; 3],
    view: &ViewFrame,
) -> ([f32; 3], [f32; 3]) {
    match orientation {
        1 => {
            let along = unit_or_unchanged(direction);
            (along, unit_or_unchanged(cross(to_particle, along)))
        }
        2 => {
            let along = unit_or_unchanged(direction);
            let world_up = [0.0, 0.0, 1.0];
            let reference = if dot(along, world_up).powi(2) > 0.99 {
                [0.0, 1.0, 0.0]
            } else {
                world_up
            };
            let side = unit_or_unchanged(cross(along, reference));
            (side, rotate_about(side, along, -1.0, 0.0))
        }
        _ => (view.right, view.up),
    }
}
