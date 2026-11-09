//! A pose made world: muted nodes, world and bind matrices, skinning rows, weapon IK and the armature.

use super::super::{
    MAX_PREVIEW_BONES, ModelPreviewData, ModelPreviewState, RenderModelPreviewNode,
};
use super::playback::{PreviewAnimationEntry, sampled_frames};
use super::pose::{blend_transforms, quat};
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
    let playback = &state.animation;
    if playback.stopped {
        return None;
    }
    let pose = playback.pose.as_ref()?;
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES || pose.frames.is_empty() {
        return None;
    }

    let (frame_a, frame_b, blend) = sampled_frames(playback, pose.frames.len());
    let (frame_a, frame_b) = (&pose.frames[frame_a], &pose.frames[frame_b]);

    let muted = variant_muted_nodes(data, state);
    let world = world_pose(nodes, |index, node| {
        if muted[index] {
            bind_transform(node)
        } else {
            blend_transforms(frame_a.get(index), frame_b.get(index), blend, node)
        }
    });
    let mut matrices: Vec<Matrix4> = world.iter().copied().map(world_matrix).collect();
    let inverse_binds: Vec<Matrix4> = nodes.iter().map(inverse_bind_matrix).collect();
    apply_weapon_ik(data, state, &mut matrices, &inverse_binds);
    // `biped_postprocess_node_matrices` (X360 0x84B1A738): the IK set, then
    // the chains.
    super::super::animation::ik::h4::apply_h4_ik_set(
        &pose.h4_ik_set,
        &data.preview.gen3.ik_markers,
        nodes,
        &inverse_binds,
        &mut matrices,
    );
    super::super::animation::ik::reach::apply_reach_ik(
        &pose.reach_ik,
        &data.preview.gen3.ik_markers,
        nodes,
        &inverse_binds,
        &mut matrices,
        sampled_frames(playback, pose.frames.len()),
    );
    let mut rows: Vec<[f32; 4]> = Vec::with_capacity(nodes.len() * 3);
    for (matrix, inverse_bind) in matrices.iter().zip(&inverse_binds) {
        let skin = *matrix * *inverse_bind;
        rows.push(skin.m[0]);
        rows.push(skin.m[1]);
        rows.push(skin.m[2]);
    }
    Some(rows)
}

/// What a Halo 2 unit's weapons lay over the clip at `index` of `clips`.
pub(super) fn h2_held_layers(
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
    let nodes = &data.preview.nodes;
    if nodes.is_empty() || nodes.len() > MAX_PREVIEW_BONES {
        return Vec::new();
    }

    let sampled_frames = (!state.animation.stopped)
        .then_some(state.animation.pose.as_ref())
        .flatten()
        .and_then(|pose| {
            if pose.frames.is_empty() {
                return None;
            }
            let (a, b, blend) = sampled_frames(&state.animation, pose.frames.len());
            Some((&pose.frames[a], &pose.frames[b], blend))
        });

    // A muted node holds its bind pose, as the skinned mesh does.
    let muted = variant_muted_nodes(data, state);
    let world = world_pose(nodes, |index, node| match sampled_frames {
        Some(_) if muted[index] => bind_transform(node),
        Some((a, b, blend)) => blend_transforms(a.get(index), b.get(index), blend, node),
        None => (
            quat(node.bind_rotation),
            RealVector3d {
                i: node.bind_translation[0],
                j: node.bind_translation[1],
                k: node.bind_translation[2],
            },
            1.0,
        ),
    });
    let mut matrices: Vec<Matrix4> = world.iter().copied().map(world_matrix).collect();
    if sampled_frames.is_some() {
        let inverse_binds: Vec<_> = nodes.iter().map(inverse_bind_matrix).collect();
        apply_weapon_ik(data, state, &mut matrices, &inverse_binds);
        if let Some(pose) = state.animation.pose.as_ref() {
            let frames = self::sampled_frames(&state.animation, pose.frames.len());
            let markers = &data.preview.gen3.ik_markers;
            super::super::animation::ik::h4::apply_h4_ik_set(
                &pose.h4_ik_set,
                markers,
                nodes,
                &inverse_binds,
                &mut matrices,
            );
            super::super::animation::ik::reach::apply_reach_ik(
                &pose.reach_ik,
                markers,
                nodes,
                &inverse_binds,
                &mut matrices,
                frames,
            );
        }
    }
    matrices.iter().map(Matrix4::translation).collect()
}
