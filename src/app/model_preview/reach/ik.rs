//! Reach's IK chains (`apply_ik_chains`), which Halo 4 solves too: after a
//! data-driven graph's animation, each chain's hand or foot is solved to a
//! marker, mostly the held weapon's, at the offset and weight its animation
//! carries for it. See `docs/reach-preview-research/ik.md`.

use std::sync::Arc;

use blam_tags::animation::{AnimationGroup, ExtendedChannel, decode_extended_block};
use blam_tags::{Skeleton, TagFile};

use crate::app::model_preview::animation::ik::solve::{
    IkHolder, IkMarker, PoseSolver, transform_point, two_bone_ik, unscaled,
};
use crate::app::model_preview::common::fields::elements;
use blam_tags::math::{Matrix4, RealPoint3d, RealQuaternion};

use crate::app::model_preview::RenderModelPreviewNode;

/// `e_animation_ik_chain_event_usage`: whose markers a target names.
const USAGE_SELF: i128 = 2;
const USAGE_PRIMARY_WEAPON: i128 = 4;
const USAGE_SECONDARY_WEAPON: i128 = 5;
/// `disable ik chains`, of an animation's `user flags`.
const DISABLE_IK_CHAINS: i128 = 1 << 11;
/// `apply_ik_chains` solves ranks 0 to 3.
const MOST_RANKS: usize = 4;
const SMALLEST_WEIGHT: f32 = 1e-4;

/// One chain's target through one clip, a value per frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReachIkTrack {
    /// The graph's chain order by start node: the order chains solve in.
    pub(crate) rank: usize,
    /// The effector node, by name.
    pub(crate) effector: String,
    pub(crate) holder: IkHolder,
    pub(crate) marker: String,
    /// The effector, in the marker's space.
    pub(crate) effector_frames: Vec<(RealQuaternion, [f32; 3])>,
    pub(crate) weights: Vec<f32>,
    /// A point the elbow or knee bends toward, in the root node's space.
    pub(crate) poles: Option<Vec<[f32; 3]>>,
}

/// One graph `ik chains` entry, its nodes by name.
struct GraphChain {
    name: String,
    kind: i128,
    start: String,
    effector: String,
    rank: usize,
}

fn node_name(skeleton: &Skeleton, index: i128) -> String {
    usize::try_from(index)
        .ok()
        .and_then(|index| skeleton.nodes.get(index))
        .map_or_else(String::new, |node| node.name.clone())
}

/// The chains of the first graph down `graphs` (the clip's own, then its
/// parents) that has any, `inherit_ik_chains_from_parent_graph` resolving
/// their nodes by name. Ranked by start node, as `post_process_ik_chains`.
fn graph_chains(graphs: &[&TagFile]) -> Vec<GraphChain> {
    for graph in graphs {
        let Some(definitions) = graph.root().descend("definitions") else {
            continue;
        };
        let skeleton = Skeleton::from_tag(graph);
        let chains = elements(definitions, "ik chains");
        if chains.is_empty() {
            continue;
        }
        let mut found: Vec<(i128, GraphChain)> = chains
            .iter()
            .map(|chain| {
                let start = chain.read_int_any("start node").unwrap_or(-1);
                let chain = GraphChain {
                    name: chain.read_string_id("name").unwrap_or_default(),
                    kind: chain.read_int_any("type").unwrap_or(0),
                    start: node_name(&skeleton, start),
                    effector: node_name(
                        &skeleton,
                        chain.read_int_any("effector node").unwrap_or(-1),
                    ),
                    rank: 0,
                };
                (start, chain)
            })
            .collect();
        found.sort_by_key(|(start, _)| *start);
        return found
            .into_iter()
            .enumerate()
            .map(|(rank, (_, chain))| GraphChain { rank, ..chain })
            .collect();
    }
    Vec::new()
}

/// What each entry of a section holds, frame by frame.
fn channels(group: &AnimationGroup<'_>, name: &str) -> Vec<ExtendedChannel> {
    group
        .data_section(name)
        .and_then(|section| decode_extended_block(section).ok())
        .unwrap_or_default()
}

/// The chain targets the clip `group` of `graphs[0]` carries
/// (`setup_ik_chain_targets_for_animation`): none if the clip turns its
/// chains off. Targets through proxies are left out; no kit unit has one.
pub(crate) fn read_reach_ik_tracks(
    graphs: &[&TagFile],
    group: &AnimationGroup<'_>,
) -> Vec<ReachIkTrack> {
    let Some(graph) = graphs.first() else {
        return Vec::new();
    };
    let Some(clip) = graph.root().descend("definitions").and_then(|definitions| {
        elements(definitions, "animations")
            .get(group.index)
            .copied()
    }) else {
        return Vec::new();
    };
    if clip
        .read_int_any("user flags")
        .is_some_and(|flags| flags & DISABLE_IK_CHAINS != 0)
    {
        return Vec::new();
    }
    let Some(shared) = elements(clip, "shared animation data").first().copied() else {
        return Vec::new();
    };
    let events = elements(shared, "ik chain events");
    if events.is_empty() {
        return Vec::new();
    }
    let chains = graph_chains(graphs);
    let skeleton = Skeleton::from_tag(graph);
    let transforms = channels(group, "ik_chain_event_data");
    let weights = channels(group, "ik_chain_control_data");
    let poles = channels(group, "ik_chain_pole_vector_data");
    // Reach's indices are signed, −1 for none; Halo 4's are unsigned bytes,
    // and its sampler takes 0xFF as none (`sample_extended_block_data`,
    // X360 0x83464668).
    let entry = |index: i128| usize::try_from(index).ok().filter(|at| *at != 0xFF);
    let mut tracks = Vec::new();
    for event in events {
        if event.read_int_any("proxy id").is_some_and(|id| id != -1) {
            continue;
        }
        let name = event.read_string_id("chain name").unwrap_or_default();
        let kind = event.read_int_any("chain type").unwrap_or(0);
        let start = node_name(
            &skeleton,
            event.read_int_any("chain start node").unwrap_or(-1),
        );
        let effector = node_name(
            &skeleton,
            event.read_int_any("chain effector node").unwrap_or(-1),
        );
        // An event naming no chain of the graph is deleted at load.
        let Some(chain) = chains.iter().find(|chain| {
            chain.name == name
                && chain.kind == kind
                && chain.start == start
                && chain.effector == effector
        }) else {
            continue;
        };
        if chain.rank >= MOST_RANKS {
            continue;
        }
        let holder = match event.read_int_any("chain usage") {
            Some(USAGE_SELF) => IkHolder::Unit,
            Some(USAGE_PRIMARY_WEAPON) => IkHolder::Weapon(1),
            Some(USAGE_SECONDARY_WEAPON) => IkHolder::Weapon(2),
            // World, parent and assassination targets have nothing to find
            // in a preview.
            _ => continue,
        };
        let transform = entry(
            event
                .read_int_any("effector transform data index")
                .unwrap_or(-1),
        );
        let effector_frames = match transform.and_then(|at| transforms.get(at..at + 2)) {
            Some(
                [
                    ExtendedChannel::Rotation(rotations),
                    ExtendedChannel::Translation(translations),
                ],
            ) => rotations
                .iter()
                .zip(translations)
                .map(|(rotation, translation)| {
                    (*rotation, [translation.x, translation.y, translation.z])
                })
                .collect(),
            _ => continue,
        };
        // With no weight a target stays at the zero it was cleared to.
        let weight = entry(
            event
                .read_int_any("effector weight data index")
                .unwrap_or(-1),
        );
        let Some(ExtendedChannel::Scalar(weights)) = weight.and_then(|at| weights.get(at)) else {
            continue;
        };
        let pole = entry(event.read_int_any("pole point data index").unwrap_or(-1));
        let poles = match pole.and_then(|at| poles.get(at)) {
            Some(ExtendedChannel::Translation(points)) => Some(
                points
                    .iter()
                    .map(|point| [point.x, point.y, point.z])
                    .collect(),
            ),
            _ => None,
        };
        tracks.push(ReachIkTrack {
            rank: chain.rank,
            effector: chain.effector.clone(),
            holder,
            marker: event.read_string_id("proxy marker").unwrap_or_default(),
            effector_frames,
            weights: weights.clone(),
            poles,
        });
    }
    tracks.sort_by_key(|track| track.rank);
    tracks
}

/// A value of `values` between frames `a` and `b`.
fn sampled<T: Copy>(values: &[T], a: usize, b: usize) -> Option<(T, T)> {
    let last = values.len().checked_sub(1)?;
    Some((values[a.min(last)], values[b.min(last)]))
}

/// Solve every chain of `tracks` on the posed `worlds`, between frames `a`
/// and `b` at `blend`. A chain whose marker is missing is skipped, where the
/// game would solve to the offset alone.
pub(crate) fn apply_reach_ik(
    tracks: &[ReachIkTrack],
    markers: &[IkMarker],
    nodes: &[RenderModelPreviewNode],
    inverse_binds: &[Matrix4],
    worlds: &mut [Matrix4],
    (a, b, blend): (usize, usize, f32),
) {
    let mut solved: Vec<usize> = Vec::new();
    for track in tracks {
        let Some((first, second)) = sampled(&track.weights, a, b) else {
            continue;
        };
        let weight = first + (second - first) * blend;
        if weight.abs() < SMALLEST_WEIGHT {
            continue;
        }
        let Some(marker) = markers
            .iter()
            .find(|marker| marker.holder == track.holder && marker.name == track.marker)
        else {
            continue;
        };
        let (Some(holder), Some(inverse)) =
            (worlds.get(marker.node), inverse_binds.get(marker.node))
        else {
            continue;
        };
        let on = *holder * *inverse * marker.bind;
        let Some(((qa, ta), (qb, tb))) = sampled(&track.effector_frames, a, b) else {
            continue;
        };
        let offset = (0..3)
            .map(|axis| ta[axis] + (tb[axis] - ta[axis]) * blend)
            .collect::<Vec<_>>();
        let point = RealPoint3d {
            x: offset[0],
            y: offset[1],
            z: offset[2],
        };
        let desired = unscaled(&on) * Matrix4::from_loc_rot_scale(point, qa.nlerp(qb, blend), 1.0);
        let Some(effector) = nodes.iter().position(|node| node.name == track.effector) else {
            continue;
        };
        if solved.contains(&effector) {
            continue;
        }
        solved.push(effector);
        let pole = track
            .poles
            .as_ref()
            .and_then(|poles| sampled(poles, a, b))
            .and_then(|(pa, pb)| {
                let local = [0, 1, 2].map(|axis| pa[axis] + (pb[axis] - pa[axis]) * blend);
                let root = worlds.first()?;
                Some(transform_point(root, local))
            });
        two_bone_ik(nodes, worlds, effector, desired, weight, pole);
    }
}

/// A clip's IK chains, solved after its IK set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReachIkChains(pub(crate) Vec<ReachIkTrack>);

/// The clip's chains, if it has any.
pub(in crate::app::model_preview) fn read_reach_ik_chains(
    graphs: &[&TagFile],
    group: &AnimationGroup<'_>,
) -> Option<Arc<dyn PoseSolver>> {
    let tracks = read_reach_ik_tracks(graphs, group);
    (!tracks.is_empty()).then(|| Arc::new(ReachIkChains(tracks)) as Arc<dyn PoseSolver>)
}

impl PoseSolver for ReachIkChains {
    fn solve(
        &self,
        markers: &[IkMarker],
        nodes: &[RenderModelPreviewNode],
        inverse_binds: &[Matrix4],
        worlds: &mut [Matrix4],
        frames: (usize, usize, f32),
    ) {
        apply_reach_ik(&self.0, markers, nodes, inverse_binds, worlds, frames);
    }

    /// Each track's frame `at` alone.
    #[cfg(test)]
    fn held_at(&self, at: usize) -> Arc<dyn PoseSolver> {
        let tracks = self
            .0
            .iter()
            .cloned()
            .map(|track| ReachIkTrack {
                effector_frames: track.effector_frames.get(at).copied().into_iter().collect(),
                weights: track.weights.get(at).copied().into_iter().collect(),
                poles: track
                    .poles
                    .map(|poles| poles.get(at).copied().into_iter().collect()),
                ..track
            })
            .collect();
        Arc::new(ReachIkChains(tracks))
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::core::game::GameId;

    /// With `BLAM_TEST_HREK`: the Spartan's rifle idle holds both hands to
    /// its primary weapon, the left arm first by rank, at full weight.
    #[test]
    fn the_spartans_rifle_idle_reaches_for_its_primary_weapon() {
        let Some(root) = kit_root(GameId::HaloReach) else {
            return;
        };
        let path = std::path::Path::new(&root)
            .join("objects/characters/spartans/spartans.model_animation_graph");
        let graph = TagFile::read(path).unwrap();
        let animation = blam_tags::Animation::new(&graph).unwrap();
        let idle = animation.find("combat:rifle:idle").unwrap();
        let tracks = read_reach_ik_tracks(&[&graph], idle);
        let summary: Vec<_> = tracks
            .iter()
            .map(|track| {
                (
                    track.rank,
                    track.effector.as_str(),
                    track.marker.as_str(),
                    track.holder,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (2, "b_l_hand", "left_hand_spartan", IkHolder::Weapon(1)),
                (3, "b_r_hand", "right_hand_spartan", IkHolder::Weapon(1)),
            ]
        );
        assert!(tracks.iter().all(|track| {
            track
                .weights
                .iter()
                .all(|weight| (weight - 1.0).abs() < 1e-3)
        }));
        assert_eq!(tracks[0].effector_frames.len(), 156);
    }
}
