//! The one place the loader names a game: what each game reads of an
//! object, a model and a Halo CE object of its own.

use super::super::RenderModelPreviewMarker;
use super::super::common::marker::MarkerFrame;
use super::super::functions::TypeExporter;
use super::super::particles::{OwnParticleSystem, ParticleMaterialReader, ParticleRules};
use super::super::{
    GameId, ModelPreviewData, RenderModelPreview, TagEntry, TagFile, TagSource,
    is_object_family_group,
};
use super::jobs::PreviewLoadSettings;
use super::object::ObjectLoad;
use super::{GameRules, LoadCx, TagReader, feature_into};
use std::sync::Arc;

/// The kit's game's load rules: its own folder's.
pub(in crate::app::model_preview) fn rules(game: Option<GameId>) -> &'static GameRules {
    match game {
        Some(GameId::HaloCe) => &super::super::ce::rules::RULES,
        Some(GameId::Halo2) => &super::super::h2::rules::RULES,
        Some(GameId::Halo3) => &super::super::h3::rules::RULES,
        Some(GameId::Halo3Odst) => &super::super::h3::rules::ODST_RULES,
        Some(GameId::HaloReach) => &super::super::reach::rules::RULES,
        Some(GameId::Halo4) => &super::super::h4::rules::RULES,
        Some(GameId::CampaignEvolved) => &super::super::campaign_evolved::rules::RULES,
        _ => &GameRules::DEFAULT,
    }
}

/// The game a model is previewed as: Halo CE's and Halo 2's by their tags'
/// generation, the Halo 3 family's by the kit, which `Game::of` cannot tell
/// apart.
pub(super) fn game_of(
    generation: blam_tags::game::Game,
    source: Option<&TagSource>,
) -> Option<GameId> {
    match generation {
        blam_tags::game::Game::Halo1 => Some(GameId::HaloCe),
        blam_tags::game::Game::Halo2 => Some(GameId::Halo2),
        blam_tags::game::Game::Halo3 => source.and_then(TagSource::game),
    }
}

/// What `object`'s type exports to its functions, where its game's
/// exporters are known: Halo 2's.
pub(in crate::app::model_preview) fn type_exports(
    object: &TagFile,
    model: Option<&TagFile>,
) -> Option<Arc<dyn TypeExporter>> {
    let exports = super::super::h2::function_exports::TypeExports::of(object, model)?;
    Some(Arc::new(exports))
}

/// The object's change colors, functions and its game's own features, each
/// noted if its reader crashes, by the kit's game.
pub(super) fn read_object_features(
    object: &ObjectLoad<'_>,
    data: &mut ModelPreviewData,
    notes: &mut Vec<String>,
) {
    let ObjectLoad {
        reader,
        object: model_tag,
        model,
    } = *object;
    let game = match &reader.source {
        TagSource::LooseFolder { game, .. } => *game,
        _ => None,
    };
    match game {
        Some(GameId::Halo3 | GameId::Halo3Odst) => {
            object.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its lights and effects", preview, |preview| {
                super::super::h3::object::read_h3_object_features(
                    reader, model_tag, model, preview,
                );
            });
        }
        // Reach's attachments, lights and effects read differently, and wait
        // on their own readers; what the object's shaders read does not.
        Some(GameId::HaloReach) => {
            object.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its flares and lights", preview, |preview| {
                super::super::reach::object::read_reach_object_features(
                    reader, model_tag, model, preview,
                );
            });
        }
        // Halo 4's functions and switches feed its materials' functions; its
        // attachments, lights and effects wait on their own readers.
        Some(GameId::Halo4) => {
            object.read_colors_and_functions(data, notes);
            super::super::h4::object::read_h4_object_features(
                reader, model_tag, model, data, notes,
            );
        }
        Some(GameId::Halo2) => {
            object.read_colors_and_functions(data, notes);
            let preview = std::sync::Arc::make_mut(&mut data.preview);
            feature_into(notes, "its lights and effects", preview, |preview| {
                super::super::h2::attached::read_h2_object_features(reader, model_tag, preview);
            });
        }
        _ => {}
    }
}

/// An attached object's own features, by its game's readers: Halo 4's and
/// Reach's own (`Game::of` calls them `Halo3` too, and they read their
/// attachments, lights and held weapons differently), the Halo 3 family's
/// with the unit's extension weapons, or Halo 2's.
pub(in crate::app::model_preview) fn read_attached_features<
    F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
>(
    cx: &LoadCx<'_, F>,
    attached: &RenderModelPreview,
    preview: &mut RenderModelPreview,
) {
    let center = || {
        std::array::from_fn(|axis| (attached.bounds_min[axis] + attached.bounds_max[axis]) * 0.5)
    };
    match cx.reader.source.game() {
        Some(GameId::Halo4) => {
            super::super::h4::attached::read_h4_attached_features(cx, center(), preview);
        }
        Some(GameId::HaloReach) => {
            super::super::reach::attached::read_reach_attached_features(cx, center(), preview);
        }
        _ if blam_tags::game::Game::of(cx.object) == blam_tags::game::Game::Halo3 => {
            super::super::h3::attached::read_h3_attached_features(cx, preview);
            super::super::weapons::attached::read_extension_weapons(cx, preview);
        }
        _ => super::super::h2::attached::read_h2_features(cx, preview),
    }
}

/// What a unit's `extension of parent` weapon, `cx`'s object, plays at the
/// unit's markers, by the weapon's own generation: the Halo 3 family's
/// attached features, or Halo 2's effects.
pub(in crate::app::model_preview) fn read_extension_weapon_features<
    F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
>(
    cx: &LoadCx<'_, F>,
    preview: &mut RenderModelPreview,
) {
    if blam_tags::game::Game::of(cx.object) == blam_tags::game::Game::Halo3 {
        super::super::h3::attached::read_h3_attached_features(cx, preview);
    } else {
        super::super::h2::attached::read_h2_extension_weapon_features(cx, preview);
    }
}

/// What a model's game reads of its own once its preview is built: Halo
/// 4's colour grading.
pub(super) fn read_model_features(reader: Option<&TagReader>, data: &mut ModelPreviewData) {
    if let Some(reader) = reader.filter(|reader| reader.source.game() == Some(GameId::Halo4)) {
        // Just built: no other handle shares it, so this does not copy.
        Arc::make_mut(&mut data.preview).h4.color_grading =
            super::super::h4::lighting::read_h4_color_grading(reader);
    }
}

/// The shaders a `.model` lays over its render model: Halo 2's hologram and
/// shield shaders, which no other game's `.model` names.
pub(super) fn add_overlay_shaders(model_tag: &TagFile, preview: &mut RenderModelPreview) {
    super::super::h2::overlays::add_overlay_shaders(model_tag, preview);
}

/// An object whose game loads it its own way: a Halo CE object, which names
/// its gbxmodel itself. `None` for any other tag.
pub(super) fn load_own_object(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Option<Result<ModelPreviewData, String>> {
    let object = is_object_family_group(
        model_tag.header.group_tag,
        reader.and_then(|reader| reader.source.game()),
    );
    (blam_tags::game::Game::of(model_tag) == blam_tags::game::Game::Halo1 && object)
        .then(|| super::super::ce::object::load_ce_object(model_tag, entry, reader, settings))
}

/// A Halo: Campaign Evolved `.model`, which names no `render model`: its
/// geometry is the UE5 SkeletalMeshes its `DA_MeshSynchronization` reaches,
/// rebuilt as the cross-game `RenderModel` for the standard pipeline. `None`
/// for any other tag.
pub(super) fn load_unreal_model(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Option<Result<ModelPreviewData, String>> {
    super::super::campaign_evolved::load_campaign_evolved_preview(
        model_tag,
        entry,
        reader,
        settings.high_detail,
    )
}

/// What a render model's generation turns before it becomes a preview: Halo
/// CE's markers.
pub(super) fn turn_render_model(
    generation: blam_tags::game::Game,
    render_model: &mut blam_tags::RenderModel,
) {
    if generation == blam_tags::game::Game::Halo1 {
        super::super::ce::render_model::turn_markers(render_model);
    }
}

/// What a render model's generation keeps of its preview: Halo CE's sorting
/// positions and shader permutations. The later games' sorting positions
/// belong to a renderer the preview does not run.
pub(super) fn finish_render_preview(
    generation: blam_tags::game::Game,
    render_tag: &TagFile,
    preview: &mut RenderModelPreview,
) {
    if generation == blam_tags::game::Game::Halo1 {
        super::super::ce::render_model::read_shader_permutations(render_tag, preview);
    } else {
        preview
            .batches
            .iter_mut()
            .for_each(|batch| batch.sort_position = None);
    }
}

/// The render model's precomputed transfer, by the fields its tag has: Halo
/// 2's `PRT info` (which Halo 2 Anniversary's self-describing tags carry
/// too, so this does not go by generation), and the Halo 3 family's
/// `per_mesh_prt_data`.
pub(super) fn read_render_prt(
    render_tag: &TagFile,
    render_model: &blam_tags::RenderModel,
    render_meshes: &[blam_tags::render_model::RenderMesh],
    preview: &mut RenderModelPreview,
) {
    preview.h2.prt = super::super::h2::prt::read_model_prt(render_tag, render_model, render_meshes)
        .filter(|prt| prt.vertices.len() == preview.vertices.len());
    // Instances draw `static_lighting_sh`, with no transfer of their own.
    let instance_vertices = preview.instances.vertex_count;
    preview.gen3.prt =
        super::super::gen3::prt::read_gen3_prt(render_tag, render_model, render_meshes)
            .map(|mut prt| {
                prt.pad(instance_vertices);
                prt
            })
            .filter(|prt| prt.vertices.len() == preview.vertices.len());
}

/// What each game's particles do differently.
impl ParticleRules {
    pub(in crate::app::model_preview) fn of(game: Option<GameId>) -> Self {
        let reach = game == Some(GameId::HaloReach);
        let halo4 = game == Some(GameId::Halo4);
        // Halo 4's simulation is Reach's, with terms of its own.
        let reach_on = reach || halo4;
        Self {
            reach_render_method: reach,
            read_material: halo4.then_some(
                super::super::h4::effects::particles::read_h4_particle_material
                    as ParticleMaterialReader,
            ),
            reach_postprocess_seeds: reach_on,
            cheap_shader: reach_on,
            adds_at_illum: reach_on,
            reach_look: reach_on,
            card_lerps: reach_on,
            random_volume_velocities: reach_on,
            scales_axes: reach_on,
            h4_emitter_terms: halo4,
            own_system: (game == Some(GameId::Halo2)).then_some(OwnParticleSystem {
                read: super::super::h2::particles::read_h2_system_parts,
                baked_lod: super::super::h2::particles::h2_baked_lod_of,
            }),
        }
    }
}
