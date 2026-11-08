//! An object held or carried by another, folded into its preview: its own
//! features by game, then its geometry, functions and change colors.

use std::sync::Arc;
use blam_tags::TagFile;
use super::super::load::merge_attached;
use crate::app::model_preview::common::marker::MarkerFrame;
use crate::app::model_preview::animation::ik::weapon::attached_marker_frame;
use super::super::common::fields::{elements, has_flag};
use super::super::dx9::passes::{h2_change_colors_for, read_h2_change_colors};
use super::super::*;
use super::super::load::LoadCx;
use super::super::h3::attached::read_h3_attached_features;
use super::super::h4::attached::read_h4_attached_features;
use super::super::reach::attached::read_reach_attached_features;
use crate::app::model_preview::common::attached::named;

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
        (attached_marker_frame(marker, &parent, &child), parent.node.max(0) as usize)
    };
    let cx = LoadCx { reader, object, owner, markers: &attached.markers, frame: &frame, name: Some(name) };
    // `Game::of` calls Reach `Halo3` too; its attachments, lights and held
    // weapons read differently, and wait on their own readers.
    if reader.source.game() == Some(GameId::Halo4) {
        // Halo 4's own readers, never Halo 3's readings of it.
        let center = std::array::from_fn(|axis| (attached.bounds_min[axis] + attached.bounds_max[axis]) * 0.5);
        read_h4_attached_features(&cx, center, preview);
    } else if reader.source.game() == Some(GameId::HaloReach) {
        let center = std::array::from_fn(|axis| (attached.bounds_min[axis] + attached.bounds_max[axis]) * 0.5);
        read_reach_attached_features(&cx, center, preview);
    } else if blam_tags::game::Game::of(object) == blam_tags::game::Game::Halo3 {
        read_h3_attached_features(&cx, preview);
        read_extension_weapons(&cx, preview);
    } else {
        super::super::h2::attached::read_h2_features(&cx, preview);
    }
    // Halo 2 lights its PRT clusters model-wide; what is attached has none here.
    if let Some(prt) = preview.h2.prt.as_mut() {
        prt.pad(attached.vertices.len());
    }
    if let Some(prt) = preview.gen3.prt.as_mut() {
        match attached.gen3.prt.as_ref().filter(|own| own.vertices.len() == attached.vertices.len()) {
            Some(own) => prt.append(own),
            None => prt.pad(attached.vertices.len()),
        }
    }
    merge_attached(preview, attached, parent, child, owner, name);
    let functions = super::super::functions::read_object_functions(object, Some(model))?;
    // `object_set_initial_change_colors`: each object its own, from its own tag.
    let change_colors = h2_change_colors_for(&read_h2_change_colors(object), variant);
    Some(AttachedObject { owner, functions, change_colors })
}

/// A unit's own `weapons` that are an `extension of parent` (vehicle and
/// turret guns): they have no model, and the marker lookup (H2 `sub_86F5B0`,
/// H3 `sub_180343CCC`, ODST `sub_140ABC7F0`) hands theirs to the unit, so
/// their effects play at the unit's markers. On a child object they take its
/// name, as its own effects do.
pub(in crate::app::model_preview) fn read_extension_weapons<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    cx: &LoadCx<'_, F>,
    preview: &mut RenderModelPreview,
) {
    let LoadCx { reader, object, owner, markers, frame, name: within } = *cx;
    let Some(holder) = super::super::common::fields::object_block(object, "weapons") else {
        return;
    };
    for entry in elements(holder, "weapons") {
        let Some(path) = entry.read_tag_ref_path("weapon").filter(|path| !path.is_empty()) else {
            continue;
        };
        let Ok(weapon) = reader.load(&path, "weapon", b"weap") else {
            continue;
        };
        let object = super::super::common::fields::object_block(&weapon, "bounding radius");
        if !object.is_some_and(|object| has_flag(object, "flags", "extension of parent")) {
            continue;
        }
        if blam_tags::game::Game::of(&weapon) == blam_tags::game::Game::Halo3 {
            read_h3_attached_features(&LoadCx { object: &weapon, ..*cx }, preview);
            continue;
        }
        let particles = super::super::particles::read_h3_particles(reader, &weapon, owner, markers, frame);
        preview.particles.extend(named(particles, within).map(Arc::new));
        let (flares, lights, gels) = super::super::h2::lights::read_h2_flares(reader, &weapon, owner, markers, frame);
        preview.h2.flares.extend(named(flares, within));
        preview.h2.point_lights.extend(named(lights, within));
        preview.h2.gel_lights.extend(named(gels, within));
        let volumes = super::super::h2::light_volumes::read_h2_light_volumes(reader, &weapon, owner, markers, frame);
        preview.h2.light_volumes.extend(named(volumes, within));
        let contrails = super::super::h2::contrails::read_h2_contrails(reader, &weapon, owner, markers, frame);
        preview.h2.contrails.extend(named(contrails, within));
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
