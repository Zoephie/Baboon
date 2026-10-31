//! What a Halo CE object brings with it at spawn. A unit's weapons are objects
//! of their own: a tank's gun is not in its model. `unit_add_initial_weapons`
//! makes them, and `unit_ready_desired_weapon` hangs the first on a marker
//! the unit's animation graph names.

use blam_tags::{TagFile, TagStruct};

use super::ce_functions::{CeObjectFunctions, read_ce_forced_permutation, read_ce_object_functions};
use super::h2_passes::{H2ChangeColors, read_h2_change_colors};
use super::loading::build_render_preview;
use super::*;
use crate::core::source::TagSource;

/// The stance a unit nobody is driving stands in.
const RESTING_SEAT: &str = "stand";

/// An object hung off the one that was opened.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeAttachedObject {
    pub(crate) name: String,
    pub(crate) functions: Option<CeObjectFunctions>,
    pub(crate) change_colors: H2ChangeColors,
}

/// A marker as a rigid transform: its three axes, then its position.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MarkerFrame {
    axes: [[f32; 3]; 3],
    position: [f32; 3],
    node: i16,
}

impl MarkerFrame {
    const ORIGIN: Self = Self {
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        position: [0.0; 3],
        node: 0,
    };

    /// `object_get_marker_by_name`: no name, or none found, is the origin.
    fn find(preview: &RenderModelPreview, name: &str) -> Self {
        preview
            .markers
            .iter()
            .find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(name))
            .map_or(Self::ORIGIN, |marker| Self {
                axes: marker.axes,
                position: marker.position,
                node: marker.node_index.max(0),
            })
    }

    /// A direction in this frame's axes, in the space the frame sits in.
    fn rotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| {
            (0..3).map(|index| self.axes[index][axis] * value[index]).sum()
        })
    }

    /// `rotate`'s inverse: axes are unit and square to each other.
    fn unrotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|index| (0..3).map(|axis| self.axes[index][axis] * value[axis]).sum())
    }
}

/// Where a point of the child goes when its `child` marker meets `parent`.
fn attach_point(parent: &MarkerFrame, child: &MarkerFrame, point: [f32; 3]) -> [f32; 3] {
    let local = child.unrotate(std::array::from_fn(|axis| point[axis] - child.position[axis]));
    let turned = parent.rotate(local);
    std::array::from_fn(|axis| turned[axis] + parent.position[axis])
}

fn attach_vector(parent: &MarkerFrame, child: &MarkerFrame, vector: [f32; 3]) -> [f32; 3] {
    parent.rotate(child.unrotate(vector))
}

fn elements<'a>(holder: TagStruct<'a>, name: &str) -> Vec<TagStruct<'a>> {
    let block = holder.field(name).and_then(|field| field.as_block());
    (0..block.as_ref().map_or(0, |block| block.len()))
        .filter_map(|index| block.as_ref()?.element(index))
        .collect()
}

/// `unit_set_or_test_seat_and_weapon_label`: every seat and weapon is tried
/// and the last that matches stands. An empty type label is for `unarmed`.
fn weapon_markers(graph: &TagFile, weapon_label: &str) -> Option<(String, String)> {
    let mut found = None;
    for seat in elements(graph.root(), "UNITS") {
        if !seat.read_string("label").is_some_and(|label| label.eq_ignore_ascii_case(RESTING_SEAT)) {
            continue;
        }
        for weapon in elements(seat, "weapons") {
            let matches = elements(weapon, "weapon types").iter().any(|kind| {
                kind.read_string("label").is_some_and(|label| label.eq_ignore_ascii_case(weapon_label))
            });
            if matches {
                found = Some((
                    weapon.read_string("hand marker").unwrap_or_default(),
                    weapon.read_string("grip marker").unwrap_or_default(),
                ));
            }
        }
    }
    found
}

/// The unit's first weapon, which it has ready, by path.
fn first_weapon(object: &TagFile) -> Option<String> {
    let mut holder = object.root();
    loop {
        if let Some(weapons) = holder.field("weapons").and_then(|field| field.as_block()) {
            return (0..weapons.len())
                .filter_map(|index| weapons.element(index)?.read_tag_ref_path("weapon"))
                .find(|path| !path.is_empty());
        }
        holder = holder.fields_all().find_map(|field| field.as_struct())?;
    }
}

/// Add `other` to `preview`, moved onto `parent` and owned by object `owner`.
fn merge_attached(
    preview: &mut RenderModelPreview,
    mut other: RenderModelPreview,
    parent: MarkerFrame,
    child: MarkerFrame,
    owner: u8,
    name: &str,
) {
    let vertex_base = preview.vertices.len() as u32;
    let index_base = preview.indices.len() as u32;
    let material_base = preview.materials.len() as u16;
    let bound = [f32::from(parent.node), 0.0, 0.0, 0.0];
    for vertex in &mut other.vertices {
        vertex.position = attach_point(&parent, &child, vertex.position);
        vertex.normal = attach_vector(&parent, &child, vertex.normal);
        vertex.tangent = attach_vector(&parent, &child, vertex.tangent);
        vertex.binormal = attach_vector(&parent, &child, vertex.binormal);
        vertex.node_indices = bound;
        vertex.node_weights = [1.0, 0.0, 0.0, 0.0];
        for axis in 0..3 {
            preview.bounds_min[axis] = preview.bounds_min[axis].min(vertex.position[axis]);
            preview.bounds_max[axis] = preview.bounds_max[axis].max(vertex.position[axis]);
        }
    }
    preview.vertices.append(&mut other.vertices);
    preview.indices.extend(other.indices.iter().map(|index| index + vertex_base));
    for material in &mut other.materials {
        material.object = owner;
    }
    preview.materials.append(&mut other.materials);
    let label = |region: &str| format!("{name}: {region}");
    for mut batch in other.batches {
        batch.index_start += index_base;
        batch.material_index += material_base;
        batch.region_name = label(&batch.region_name);
        preview.batches.push(batch);
    }
    for mut region in other.regions {
        region.name = label(&region.name);
        preview.regions.push(region);
    }
}

/// Hang the unit's ready weapon on it, returning the object that adds.
pub(super) fn attach_ce_weapon(
    source: &TagSource,
    object: &TagFile,
    preview: &mut RenderModelPreview,
) -> Option<CeAttachedObject> {
    let path = first_weapon(object)?;
    let weapon = load_referenced_tag_from_source(source, &path, "weapon", b"weap").ok()?;
    let model = object_reference(&weapon, "model")?;
    let render = load_referenced_tag_from_source(source, &model, "gbxmodel", b"mod2").ok()?;
    let mut attached = build_render_preview(&render).ok()?;
    let forced = read_ce_forced_permutation(&weapon);
    if forced != 0 {
        attached.materials.iter_mut().for_each(|material| material.permutation = forced);
    }
    let label = weapon.root().read_string("label").unwrap_or_default();
    let (hand, grip) = object_reference(object, "animation graph")
        .and_then(|graph| {
            load_referenced_tag_from_source(source, &graph, "model_animations", b"antr").ok()
        })
        .and_then(|graph| weapon_markers(&graph, &label))
        .unwrap_or_default();
    let parent = MarkerFrame::find(preview, &hand);
    let child = MarkerFrame::find(&attached, &grip);
    let name = tag_path::name(&path).to_owned();
    merge_attached(preview, attached, parent, child, 1, &name);
    Some(CeAttachedObject {
        name,
        functions: read_ce_object_functions(&weapon),
        change_colors: read_h2_change_colors(&weapon),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turned() -> MarkerFrame {
        // A quarter turn about up, set off along x.
        MarkerFrame {
            axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            position: [2.0, 0.0, 0.0],
            node: 3,
        }
    }

    #[test]
    fn a_grip_lands_on_the_hand_with_its_axes() {
        let child = MarkerFrame { position: [0.0, 0.0, 1.0], ..MarkerFrame::ORIGIN };
        assert_eq!(attach_point(&turned(), &child, [0.0, 0.0, 1.0]), [2.0, 0.0, 0.0]);
        assert_eq!(attach_point(&turned(), &child, [1.0, 0.0, 1.0]), [2.0, 1.0, 0.0]);
        assert_eq!(attach_vector(&turned(), &child, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
    }

    #[test]
    fn no_marker_name_is_the_origin() {
        let mut preview = RenderModelPreview::default();
        preview.markers.push(RenderModelPreviewMarker {
            name: "Gun".to_owned(),
            node_index: 2,
            position: [1.0, 2.0, 3.0],
            axes: MarkerFrame::ORIGIN.axes,
        });
        assert_eq!(MarkerFrame::find(&preview, ""), MarkerFrame::ORIGIN);
        assert_eq!(MarkerFrame::find(&preview, "missing"), MarkerFrame::ORIGIN);
        assert_eq!(MarkerFrame::find(&preview, "gun").node, 2);
    }

    #[test]
    fn a_merged_model_keeps_its_own_indices_materials_and_owner() {
        let mut preview = RenderModelPreview {
            vertices: vec![RenderModelPreviewVertex::default(); 3],
            indices: vec![0, 1, 2],
            materials: vec![RenderModelPreviewMaterial::default()],
            ..Default::default()
        };
        let other = RenderModelPreview {
            vertices: vec![RenderModelPreviewVertex::default(); 3],
            indices: vec![0, 2, 1],
            materials: vec![RenderModelPreviewMaterial::default()],
            regions: vec![RenderModelPreviewRegion { name: "body".to_owned(), permutations: Vec::new() }],
            batches: vec![RenderModelPreviewBatch {
                region_name: "body".to_owned(),
                index_count: 3,
                ..Default::default()
            }],
            ..Default::default()
        };
        merge_attached(&mut preview, other, turned(), MarkerFrame::ORIGIN, 1, "gun");
        assert_eq!(preview.indices, [0, 1, 2, 3, 5, 4]);
        let batch = &preview.batches[0];
        assert_eq!((batch.index_start, batch.material_index), (3, 1));
        assert_eq!(batch.region_name, "gun: body");
        assert_eq!(preview.regions[0].name, "gun: body");
        assert_eq!(preview.materials[1].object, 1);
        assert_eq!(preview.vertices[3].node_indices[0], 3.0);
        assert_eq!(preview.vertices[3].node_weights, [1.0, 0.0, 0.0, 0.0]);
    }

    /// With `BLAM_TEST_HCEEK`: the kit's markers, by the Chief's rifle and the
    /// Warthog's gun, and the empty pair a tank's gun leaves.
    #[test]
    fn a_unit_finds_its_weapons_markers_in_its_animations() {
        let Ok(root) = std::env::var("BLAM_TEST_HCEEK") else {
            eprintln!("skipping: BLAM_TEST_HCEEK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloCe),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let graph = |path: &str| {
            load_referenced_tag_from_source(&source, path, "model_animations", b"antr").unwrap()
        };
        let markers = |path: &str, label: &str| weapon_markers(&graph(path), label);
        let hand = |hand: &str| Some((hand.to_owned(), String::new()));
        assert_eq!(markers("characters\\cyborg\\cyborg", "ar"), hand("right hand"));
        assert_eq!(markers("vehicles\\warthog\\warthog", "cg"), hand("gun"));
        assert_eq!(markers("vehicles\\scorpion\\scorpion", "cannon"), hand(""));
        assert_eq!(markers("vehicles\\scorpion\\scorpion", "nothing"), None);
    }
}
