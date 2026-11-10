//! An object held or carried by another, folded into its preview: its own
//! features by its game's readers, then its geometry, functions and change colors.

use super::super::common::fields::{elements, has_flag};
use super::super::functions::{change_colors_for, read_change_colors};
use super::super::load::LoadCx;
use super::super::load::{merge_attached, read_attached_features, read_extension_weapon_features};
use super::super::{RenderModelPreview, RenderModelPreviewMarker, TagReader};
use crate::app::model_preview::animation::ik::weapon::attached_marker_frame;
use crate::app::model_preview::common::marker::MarkerFrame;
use blam_tags::TagFile;

/// An attached object's lights and flares, then its geometry, into `preview`.
/// `model` is the object's, built into `attached`, made as `variant`.
#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) fn fold_attached(
    reader: &TagReader,
    object: &TagFile,
    model: &TagFile,
    variant: &str,
    attached: RenderModelPreview,
    parent: MarkerFrame,
    child: MarkerFrame,
    owner: u8,
    name: &str,
    preview: &mut RenderModelPreview,
) -> Option<AttachedObject> {
    let frame = |marker: &RenderModelPreviewMarker| {
        (
            attached_marker_frame(marker, &parent, &child),
            parent.node.max(0) as usize,
        )
    };
    let cx = LoadCx {
        reader,
        object,
        owner,
        markers: &attached.markers,
        frame: &frame,
        name: Some(name),
    };
    read_attached_features(&cx, &attached, preview);
    // The per-vertex streams stay parallel: the attached object's own where
    // its game keeps them per object, else none.
    for stream in super::super::load::vertex_streams() {
        stream.append_attached(preview, &attached, attached.vertices.len());
    }
    merge_attached(preview, attached, parent, child, owner, name);
    let functions = super::super::functions::read_object_functions(object, Some(model))?;
    // `object_set_initial_change_colors`: each object its own, from its own tag.
    let change_colors = change_colors_for(&read_change_colors(object), variant);
    Some(AttachedObject {
        owner,
        functions,
        change_colors,
    })
}

/// A unit's own `weapons` that are an `extension of parent` (vehicle and
/// turret guns): they have no model, and the marker lookup (H2 `sub_86F5B0`,
/// H3 `sub_180343CCC`, ODST `sub_140ABC7F0`) hands theirs to the unit, so
/// their effects play at the unit's markers. On a child object they take its
/// name, as its own effects do.
pub(in crate::app::model_preview) fn read_extension_weapons<
    F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
>(
    cx: &LoadCx<'_, F>,
    preview: &mut RenderModelPreview,
) {
    let LoadCx { reader, object, .. } = *cx;
    let Some(holder) = super::super::common::fields::object_block(object, "weapons") else {
        return;
    };
    for entry in elements(holder, "weapons") {
        let Some(path) = entry
            .read_tag_ref_path("weapon")
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        let Ok(weapon) = reader.load(&path, "weapon", b"weap") else {
            continue;
        };
        let object = super::super::common::fields::object_block(&weapon, "bounding radius");
        if !object.is_some_and(|object| has_flag(object, "flags", "extension of parent")) {
            continue;
        }
        let weapon_cx = LoadCx {
            object: &weapon,
            ..*cx
        };
        read_extension_weapon_features(&weapon_cx, preview);
    }
}

/// An object attached to the opened one, in any game from Halo 2 on: a held
/// weapon, or what one of its variants attaches.
#[derive(Debug, Clone)]
pub(crate) struct AttachedObject {
    /// What its materials carry as `object`.
    pub(crate) owner: u8,
    pub(crate) functions: super::super::functions::ObjectFunctions,
    pub(crate) change_colors: [[f32; 3]; 4],
}
