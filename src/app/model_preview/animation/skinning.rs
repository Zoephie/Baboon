//! A pose made world: muted nodes, world and bind matrices, skinning rows, weapon IK and the armature.

use super::super::{
    MAX_PREVIEW_BONES, ModelPreviewData, ModelPreviewState, RenderModelPreviewNode,
};
use super::playback::{PreviewAnimationEntry, sampled_frames};
use super::pose::blend_transforms;
use blam_tags::math::{Matrix4, RealPoint3d, RealQuaternion, RealVector3d};

/// Which nodes a variant's `muted nodes` hold: each named node and its whole
/// subtree (`model_postprocess_muted_nodes`, X360 0x82D96890, through
/// `model_set_muted_flag_subtree`). Nodes come parents first.
pub(in crate::app::model_preview) fn muted_nodes(
    nodes: &[RenderModelPreviewNode],
    names: &[String],
) -> Vec<bool> {
    let mut muted = vec![false; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let parent_muted = usize::try_from(node.parent)
            .ok()
            .and_then(|parent| muted.get(parent))
            .copied()
            .unwrap_or(false);
        muted[index] = parent_muted || names.contains(&node.name);
    }
    muted
}

/// The selected variant's [`muted_nodes`]; none with no variant selected.
fn variant_muted_nodes(data: &ModelPreviewData, state: &ModelPreviewState) -> Vec<bool> {
    let nodes = &data.preview.nodes;
    state
        .selected_variant
        .and_then(|index| data.variants.get(index))
        .map_or_else(
            || vec![false; nodes.len()],
            |variant| muted_nodes(nodes, &variant.muted_nodes),
        )
}

/// Each node's world rotation, translation and scale, from the local
/// transform `local` gives it: parents first, as the nodes come.
fn world_pose(
    nodes: &[RenderModelPreviewNode],
    mut local: impl FnMut(usize, &RenderModelPreviewNode) -> (RealQuaternion, RealVector3d, f32),
) -> Vec<(RealQuaternion, RealVector3d, f32)> {
    let mut world: Vec<(RealQuaternion, RealVector3d, f32)> = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let local = local(index, node);
        let transform = if node.parent >= 0 {
            let (parent_rotation, parent_translation, parent_scale) = world
                .get(node.parent as usize)
                .copied()
                .unwrap_or((RealQuaternion::IDENTITY, RealVector3d::ZERO, 1.0));
            (
                (parent_rotation * local.0).normalized(),
                parent_translation + (parent_rotation * (local.1 * parent_scale)),
                parent_scale * local.2,
            )
        } else {
            local
        };
        world.push(transform);
    }
    world
}

/// A muted node's local transform (`object_handle_muted_nodes`, X360
/// 0x84A476D0): the render model node's default rotation and translation,
/// scale 1.
fn bind_transform(node: &RenderModelPreviewNode) -> (RealQuaternion, RealVector3d, f32) {
    // With no frame either side, the blend is the bind pose.
    blend_transforms(None, None, 0.0, node)
}

/// Sample the playback state into skinning-matrix rows for this draw frame —
/// three vec4 rows per preview node, `world × inverse_bind`, so an
/// unanimated node lands exactly on its bind pose. `None` draws the plain
/// bind pose.
pub(in crate::app::model_preview) fn animation_skinning_rows(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Option<Vec<[f32; 4]>> {
    let matrices = posed_node_matrices(data, state)?;
    let mut rows: Vec<[f32; 4]> = Vec::with_capacity(matrices.len() * 3);
    for (matrix, node) in matrices.iter().zip(&data.preview.nodes) {
        let skin = *matrix * inverse_bind_matrix(node);
        rows.push(skin.m[0]);
        rows.push(skin.m[1]);
        rows.push(skin.m[2]);
    }
    Some(rows)
}

/// Each node's world transform as the preview poses it: [`pose_beneath`],
/// with the hand pose laid over it. `None` while nothing moves a node from
/// its bind pose.
pub(in crate::app::model_preview) fn posed_node_matrices(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Option<Vec<Matrix4>> {
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES {
        return None;
    }
    let posed = state.pose.geometry == data.geometry_id && state.pose.is_posed();
    let animated = animated_node_matrices(data, state);
    if animated.is_none() && !posed {
        return None;
    }
    let mut matrices = animated.unwrap_or_else(|| bind_node_matrices(nodes));
    if posed {
        super::super::pose::apply(nodes, &mut matrices, &state.pose);
    }
    Some(matrices)
}

/// Each node's world transform beneath the hand pose: the animation's, or
/// the bind pose. Empty for a skeleton the preview draws no armature for.
pub(in crate::app::model_preview) fn pose_beneath(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<Matrix4> {
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES {
        return Vec::new();
    }
    animated_node_matrices(data, state).unwrap_or_else(|| bind_node_matrices(nodes))
}

fn bind_node_matrices(nodes: &[RenderModelPreviewNode]) -> Vec<Matrix4> {
    world_pose(nodes, |_, node| bind_transform(node))
        .into_iter()
        .map(world_matrix)
        .collect()
}

/// Each node's world transform as the animation playing poses it, with the
/// game's IK (`biped_postprocess_node_matrices`, X360 0x84B1A738: the
/// weapon's, the IK set, then the chains). `None` with no animation playing.
fn animated_node_matrices(data: &ModelPreviewData, state: &ModelPreviewState) -> Option<Vec<Matrix4>> {
    let nodes = &data.preview.nodes;
    let playback = &state.animation;
    let pose = (!playback.stopped)
        .then_some(playback.pose.as_ref())
        .flatten()
        .filter(|pose| !pose.frames.is_empty())?;
    let frames = sampled_frames(playback, pose.frames.len());
    let (frame_a, frame_b) = (&pose.frames[frames.0], &pose.frames[frames.1]);
    // A muted node holds its bind pose.
    let muted = variant_muted_nodes(data, state);
    let world = world_pose(nodes, |index, node| {
        if muted[index] {
            bind_transform(node)
        } else {
            blend_transforms(frame_a.get(index), frame_b.get(index), frames.2, node)
        }
    });
    let mut matrices: Vec<Matrix4> = world.into_iter().map(world_matrix).collect();
    let inverse_binds: Vec<Matrix4> = nodes.iter().map(inverse_bind_matrix).collect();
    apply_weapon_ik(data, state, &mut matrices, &inverse_binds);
    for solver in &pose.solvers {
        solver.solve(&data.preview.gen3.ik_markers, nodes, &inverse_binds, &mut matrices, frames);
    }
    Some(matrices)
}

/// What a Halo 2 unit's weapons lay over the clip at `index` of `clips`.
pub(super) fn held_layers(
    clips: &[PreviewAnimationEntry],
    index: usize,
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<super::super::weapons::PoseLayer> {
    let (Some(weapons), Some(clip)) = (data.weapons.as_ref(), clips.get(index)) else {
        return Vec::new();
    };
    let names: Vec<_> = clips.iter().map(|clip| clip.name.as_str()).collect();
    let (primary, secondary) = (state.weapon.as_deref(), state.secondary_weapon.as_deref());
    weapons.pose_layers(&names, &clip.name, primary, secondary)
}

fn world_matrix((rotation, translation, scale): (RealQuaternion, RealVector3d, f32)) -> Matrix4 {
    Matrix4::from_loc_rot_scale(
        RealPoint3d {
            x: translation.i,
            y: translation.j,
            z: translation.k,
        },
        rotation,
        scale,
    )
}

fn inverse_bind_matrix(node: &RenderModelPreviewNode) -> Matrix4 {
    Matrix4 {
        m: [
            node.inverse_bind[0],
            node.inverse_bind[1],
            node.inverse_bind[2],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

/// Hold a unit's hands to its weapon, for the clips the game does.
fn apply_weapon_ik(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
    matrices: &mut [Matrix4],
    inverse_binds: &[Matrix4],
) {
    let clip = state
        .animation
        .selected
        .and_then(|index| data.animations.as_ref()?.get(index))
        .map(|entry| entry.name.as_str());
    let Some(clip) = clip else {
        return;
    };
    for point in &data.preview.weapon_ik {
        let runs = match &data.weapons {
            Some(weapons) => weapons.ik_runs_through(clip, &point.scope),
            None => point.runs_through(clip),
        };
        if runs {
            point.apply(matrices, inverse_binds);
        }
    }
}

/// World-space joint positions for the armature overlay. This follows the
/// same sampled parent-local pose as GPU skinning, and falls back to the bind
/// pose when no animation is active.
pub(in crate::app::model_preview) fn armature_node_positions(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<[f32; 3]> {
    armature_node_matrices(data, state)
        .iter()
        .map(Matrix4::translation)
        .collect()
}

/// Each node's world transform as the preview poses it, as the armature is
/// drawn: [`posed_node_matrices`], or the bind pose.
pub(in crate::app::model_preview) fn armature_node_matrices(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<Matrix4> {
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES {
        return Vec::new();
    }
    posed_node_matrices(data, state).unwrap_or_else(|| bind_node_matrices(nodes))
}
