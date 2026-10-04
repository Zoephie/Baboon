//! Preview tag loading and render-model resolution.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use crate::core::source::MountedContainer;
use blam_tags::iostore::IoStoreArchive;
use blam_tags::iostore::container_header::EIoContainerHeaderVersion;
use blam_tags::iostore::skeletal_mesh::SkeletalMesh;
use blam_tags::iostore::static_mesh::StaticMesh;
use blam_tags::iostore::ue_types::{EIoStoreTocVersion, FPackageObjectIndex};
use blam_tags::iostore::unversioned::{
    MeshRef, MeshSyncRegions, Permutation, PropValue, PropertyBlock, Region,
};
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::zen::FZenPackageHeader;
use blam_tags::jms::{UeMeshPart, UeStaticPart, UeWorldPart};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, LazyLock, Mutex};

const CE_CV: EIoStoreTocVersion = EIoStoreTocVersion::ReplaceIoChunkHashWithIoHash;
const CE_HV: EIoContainerHeaderVersion = EIoContainerHeaderVersion::SoftPackageReferences;

/// The load-affecting slice of [`ModelPreviewState`], snapshotted before the
/// parse so the loader never holds the mutable state alongside it.
#[derive(Clone, Default)]
pub(super) struct PreviewLoadSettings {
    pub(super) high_detail: bool,
    pub(super) scenario_selection: std::collections::BTreeSet<usize>,
}

impl PreviewLoadSettings {
    fn of(state: &ModelPreviewState) -> Self {
        Self {
            high_detail: state.high_detail,
            scenario_selection: state.scenario_bsp_selection.clone(),
        }
    }
}

static NEXT_MODEL_PREVIEW_LOAD_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

impl Baboon {
    /// Start the expensive base geometry + variant build after the loading
    /// cards have had a frame to appear. Re-reading the tag on the worker is
    /// intentional: `TagFile` is not cloneable, and moving the open document
    /// away would make its Fields tab unavailable while the preview builds.
    pub(in crate::app) fn maybe_request_model_preview(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) {
        let kit = &self.model.kits[kit_index];
        let view = &self.views[kit.id];
        let Some(state) = view.caches.model_previews.get(key) else {
            return;
        };
        if state.active_tab != ModelTagPanelTab::ModelPreview {
            return;
        }
        let desired_matches = state.loaded_key.as_deref() == Some(key)
            && state.loaded_high_detail == state.high_detail
            && state.loaded_scenario_selection == state.scenario_bsp_selection;
        if desired_matches && (state.data.is_some() || state.preview_load_id.is_some()) {
            return;
        }

        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return;
        };
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return;
        };
        // Preserve unsaved model/variant edits. Clean documents are re-read on
        // the worker so a large BSP does not have to serialize on the UI
        // thread merely to hand the same bytes back to the parser.
        let edited_model_bytes = kit
            .parsed_tags
            .get(key)
            .filter(|document| document.dirty.is_set())
            .map(|document| {
                document
                    .tag
                    .write_to_bytes()
                    .map_err(|error| format!("Could not snapshot edited model: {error}"))
            });
        // Edited bytes re-parse the way the kit reads them from disk: a
        // classic (H2/CE) tag has no self-describing layout for
        // `TagFile::read_from_bytes` to find.
        let (game, definitions_root) = match &source {
            TagSource::LooseFolder {
                game,
                definitions_root,
                ..
            } => (game.clone(), Some(definitions_root.clone())),
            _ => (None, None),
        };
        let group_tag = entry.group_tag;
        let names = kit.names.clone();
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        let settings = PreviewLoadSettings::of(state);
        let request_id =
            NEXT_MODEL_PREVIEW_LOAD_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let state = self.views[self.model.kits[kit_index].id]
            .caches.model_previews
            .get_mut(key)
            .expect("preview state checked above");
        state.loaded_key = Some(key.to_owned());
        state.loaded_high_detail = settings.high_detail;
        state.loaded_scenario_selection = settings.scenario_selection.clone();
        state.preview_load_id = Some(request_id);
        state.data = None;
        state.render_model_path = None;
        // A fresh base load orphans any overlay, material, or animation work
        // keyed to the old geometry. Their request hooks re-arm after this
        // worker result is accepted.
        state.overlays_pending = false;
        state.overlays_loaded = false;
        state.textures_pending = false;
        state.animation = PreviewAnimationPlayback::default();

        let (worker_key, panic_key) = (key.to_owned(), key.to_owned());
        let load = move || -> Result<_, String> {
                let model_tag = match edited_model_bytes {
                    Some(Ok(bytes)) => crate::core::source::read_tag_from_bytes(
                        &bytes,
                        game,
                        definitions_root.as_deref(),
                        group_tag,
                    )
                    .map_err(|error| error.to_string())?,
                    Some(Err(error)) => return Err(error),
                    None => read_entry(&source, &entry).map_err(|error| error.to_string())?,
                };
                load_model_preview(&model_tag, &entry, &names, Some(&source), &settings)
        };
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::ModelPreviewLoaded {
                stamp,
                key: worker_key,
                request_id,
                result: load(),
            },
            move |_| WorkerMessage::ModelPreviewLoaded {
                stamp,
                key: panic_key,
                request_id,
                result: Err("Render model preview crashed while parsing this tag.".to_owned()),
            },
        );
    }

    pub(in crate::app) fn handle_model_preview_loaded(
        &mut self,
        stamp: KitStamp,
        key: String,
        request_id: u64,
        result: Result<ModelPreviewData, String>,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.model.resolve_stamp(stamp).is_none();
        let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(&key) else {
            return true;
        };
        if state.preview_load_id != Some(request_id) {
            return true;
        }
        // The request is answered before the staleness check. A result dropped
        // for a generation bump used to leave the id set, and with no data and
        // a request "in flight" nothing asked again: the pane sat on its
        // loading shells, repainting every frame, until the tag was closed.
        state.preview_load_id = None;
        if stale {
            return true;
        }
        if let Ok(data) = &result {
            state.render_model_path = Some(data.render_model_path.clone());
            // Auto-select the canonical variant (named `default`, else the
            // first) so both cards arrive in one consistent state.
            let default_variant = default_variant_index(&data.variants);
            reset_model_preview_selection(state, data, default_variant);
        }
        state.data = Some(result);
        false
    }
}

pub(super) fn load_model_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    let high_detail = settings.high_detail;
    // `particle_model` carries its geometry inline with no wrapper, but
    // it is not a render_model: no regions, no permutations, no
    // materials, no nodes. Its objects are the entries of the JMI the
    // artist imported, so each becomes a preview region and gets its own
    // toggle in the region list.
    if blam_tags::is_particle_model_group(model_tag.header.group_tag) {
        let stem = preview_tag_stem(entry);
        let preview = build_particle_model_preview(model_tag, &stem)?;
        if preview.batches.is_empty() {
            return Err("This particle_model has no previewable geometry.".to_owned());
        }
        return Ok(model_preview_data(
            entry.key.clone(),
            entry.display_path.clone(),
            preview,
            Vec::new(),
        ));
    }

    // Halo CE `gbxmodel` (mod2) and a bare `render_model` (mode) ARE the
    // render geometry — there is no `.model` (hlmt) wrapper carrying a
    // "render model" reference, so preview the tag itself.
    let group = model_tag.header.group_tag.to_be_bytes();
    if matches!(&group, b"mode" | b"mod2") {
        let preview = build_render_preview(model_tag)?;
        if preview.batches.is_empty() {
            return Err("This render tag has no previewable draw batches.".to_owned());
        }
        return Ok(model_preview_data(
            entry.key.clone(),
            entry.display_path.clone(),
            preview,
            Vec::new(),
        ));
    }

    // Halo CE object-family tags are the equivalent of later engines' model
    // wrapper: the object directly names its gbxmodel, collision, animation,
    // and legacy physics tags. Start with the render reference here; collision
    // is added by the background overlay path.
    if blam_tags::game::Game::of(model_tag) == blam_tags::game::Game::Halo1
        && is_object_family_group(model_tag.header.group_tag)
    {
        let Some(source) = source else {
            return Err("Halo CE object preview requires a loaded source.".to_owned());
        };
        let reference = halo1_object_reference(model_tag, "model")
            .ok_or("This object references no gbxmodel.")?;
        let render = load_referenced_tag_from_source(source, &reference, "gbxmodel", b"mod2")
            .map_err(|error| format!("Could not load {reference}.gbxmodel: {error}"))?;
        let preview = build_render_preview(&render)?;
        if preview.batches.is_empty() {
            return Err("Referenced gbxmodel has no previewable draw batches.".to_owned());
        }
        return Ok(model_preview_data(
            entry.key.clone(),
            reference,
            preview,
            Vec::new(),
        ));
    }

    // Collision and physics tags carry no render geometry of their own —
    // their preview is derived: collision BSPs walked into triangles, physics
    // primitives tessellated. Posed on the owning `.model`'s skeleton when
    // one is beside them, unposed otherwise.
    if matches!(&group, b"coll" | b"phmo") {
        let skeleton = source.and_then(|source| owning_model_skeleton(source, entry));
        let nodes = skeleton.as_ref().map(|skeleton| skeleton.nodes());
        let preview = if &group == b"coll" {
            build_collision_preview(model_tag, nodes)?
        } else {
            build_physics_preview(model_tag, nodes)?
        };
        return Ok(model_preview_data(
            entry.key.clone(),
            entry.display_path.clone(),
            preview,
            Vec::new(),
        ));
    }

    // A structure BSP previews its own geometry, layered into `render` /
    // `collision` / `portals` / `weather` regions so each is a toggle.
    if &group == b"sbsp" {
        let preview = build_sbsp_preview(model_tag, false)?;
        return Ok(model_preview_data(
            entry.key.clone(),
            entry.display_path.clone(),
            preview,
            Vec::new(),
        ));
    }

    // A scenario previews a composite of the structure BSPs the user has
    // checked — each selected BSP loads render-only and lands as one region,
    // so the region list doubles as the per-BSP toggle. Nothing loads until
    // something is checked: a campaign scenario's full BSP set is far too
    // much to parse unasked on the UI thread.
    if &group == b"scnr" {
        let bsps = scenario_bsp_paths(model_tag);
        if bsps.is_empty() {
            return Err("This scenario lists no structure BSPs.".to_owned());
        }
        let mut preview = RenderModelPreview::default();
        if !settings.scenario_selection.is_empty() {
            let Some(source) = source else {
                return Err("Scenario preview requires a loaded source.".to_owned());
            };
            preview.bounds_min = [f32::INFINITY; 3];
            preview.bounds_max = [f32::NEG_INFINITY; 3];
            for &index in &settings.scenario_selection {
                let Some(Some(reference)) = bsps.get(index) else {
                    continue;
                };
                let bsp_tag = load_referenced_tag_from_source(
                    source,
                    reference,
                    "scenario_structure_bsp",
                    b"sbsp",
                )
                .map_err(|error| format!("Could not load {reference}: {error}"))?;
                let mut bsp_preview = build_sbsp_preview(&bsp_tag, true)
                    .map_err(|error| format!("{reference}: {error}"))?;
                rebrand_preview_region(&mut bsp_preview, &bsp_display_name(reference));
                merge_preview_append(&mut preview, &bsp_preview);
            }
            if !preview.bounds_min.iter().all(|bound| bound.is_finite()) {
                preview.bounds_min = [0.0; 3];
                preview.bounds_max = [0.0; 3];
            }
        }
        let mut data = model_preview_data(
            entry.key.clone(),
            entry.display_path.clone(),
            preview,
            Vec::new(),
        );
        data.scenario_bsps = bsps;
        return Ok(data);
    }

    // Halo: Campaign Evolved — the `.model` (hlmt) has no `render model` ref;
    // geometry lives in UE5 SkeletalMeshes reached via `DA_MeshSynchronization`.
    // Reconstruct the cross-game RenderModel and feed the standard pipeline.
    if let Some(result) = load_campaign_evolved_preview(model_tag, entry, source, high_detail) {
        return result;
    }

    let Some((group_tag, rel_path)) = model_tag.root().read_tag_ref_with_group("render model")
    else {
        return Err("This model tag has no render model reference.".to_owned());
    };
    if rel_path.trim().is_empty() {
        return Err("This model tag has an empty render model reference.".to_owned());
    }
    let Some(TagSource::LooseFolder { root, .. }) = source else {
        return Err("Render model preview requires a loaded loose-folder editing kit.".to_owned());
    };
    let extension = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
        .unwrap_or("render_model");
    let mut normalized = rel_path.replace('/', "\\");
    if let Some(stripped) = normalized.strip_suffix(&format!(".{extension}")) {
        normalized = stripped.to_owned();
    }
    let path = resolve_tag_path(root, &normalized, extension);
    if !path.exists() {
        return Err(format!(
            "Referenced render_model was not found: {}",
            path.display()
        ));
    }
    let render_entry = TagEntry {
        key: file_entry_key(&path),
        display_path: format!("{}.{}", normalized.replace('\\', "/"), extension),
        group_tag,
        group_name: names.name_for(group_tag).map(str::to_owned),
        location: TagEntryLocation::LooseFile(path),
    };
    let render_tag =
        read_entry(source.unwrap(), &render_entry).map_err(|error| error.to_string())?;
    let preview = build_render_preview(&render_tag)?;
    if preview.batches.is_empty() {
        return Err("Referenced render_model has no previewable draw batches.".to_owned());
    }
    // The collision/physics overlays are NOT built here: this parse runs on
    // the UI thread, and walking the collision BSP plus tessellating the
    // physics shapes froze the frame every time a toggle rebuilt the merge.
    // `maybe_request_model_overlays` builds them once on a worker after this
    // load lands, and the toggles are draw-time filters from then on.
    Ok(model_preview_data(
        render_entry.key,
        normalized,
        preview,
        read_model_variants(model_tag),
    ))
}

/// Build preview geometry from a render-geometry tag — a `render_model`
/// (`mode`), a Halo CE `gbxmodel` (`mod2`), or a Halo 2 `render_model`.
///
/// One tag→geometry path for every engine: blam-tags' `RenderModel::from_tag`
/// / `derive_render_meshes` game-dispatch (H3 reads `render geometry`, H2 the
/// `sections`, Halo CE the gbxmodel `geometries`), so batches carry the render
/// model's own region/permutation names and stay in sync with the variant
/// selection. JMS is export-only — never used for rendering.
/// Tag basename, used to name gen3 particle objects (`pmdf` stores no
/// object names — see `blam_tags::particle_model`). Falls back to the
/// entry key when the display path has no usable leaf.
fn preview_tag_stem(entry: &TagEntry) -> String {
    entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .map(|leaf| leaf.split('.').next().unwrap_or(leaf))
        .filter(|stem| !stem.is_empty())
        .unwrap_or("particle_model")
        .to_owned()
}

/// Build preview geometry from a `particle_model` (`pmdf` or Halo 2
/// `PRTM`).
///
/// blam-tags does the decode — splitting the merged strip at the
/// `m_gpu_data/m_variants` boundaries (gen3) or the `models[]` ranges
/// (Halo 2), decompressing positions through the compression bounds, and
/// compacting each object to its own vertex set. Each object lands as a
/// one-permutation region so the existing region list doubles as an
/// object toggle, which is the closest thing the tag has to structure.
pub(super) fn build_particle_model_preview(
    tag: &TagFile,
    stem: &str,
) -> Result<RenderModelPreview, String> {
    let meshes = blam_tags::particle_model_meshes(tag, stem).map_err(|e| e.to_string())?;

    let mut preview = RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };

    for mesh in &meshes {
        let Ok(vertex_base) = u32::try_from(preview.vertices.len()) else {
            continue;
        };
        if mesh
            .vertices
            .len()
            .checked_add(preview.vertices.len())
            .is_none_or(|count| count > u32::MAX as usize)
        {
            continue;
        }

        preview.vertices.reserve(mesh.vertices.len());
        for vertex in &mesh.vertices {
            let position = [vertex.position.x, vertex.position.y, vertex.position.z];
            expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
            preview.vertices.push(RenderModelPreviewVertex {
                position,
                normal: [vertex.normal.i, vertex.normal.j, vertex.normal.k],
                // Particle models carry no tangent frame, and this path has no
                // materials to texture with — the defaults leave it unshaded.
                ..Default::default()
            });
        }

        let Ok(index_start) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        preview
            .indices
            .extend(mesh.indices.iter().map(|index| vertex_base + index));
        let Ok(index_end) = u32::try_from(preview.indices.len()) else {
            continue;
        };
        let index_count = index_end - index_start;
        if index_count == 0 {
            continue;
        }

        preview.regions.push(RenderModelPreviewRegion {
            name: mesh.name.clone(),
            permutations: vec![PARTICLE_OBJECT_PERMUTATION.to_owned()],
        });
        preview.batches.push(RenderModelPreviewBatch {
            region_name: mesh.name.clone(),
            permutation_name: PARTICLE_OBJECT_PERMUTATION.to_owned(),
            material_index: 0,
            index_start,
            index_count,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
        });
    }

    if preview.vertices.is_empty() {
        preview.bounds_min = [0.0; 3];
        preview.bounds_max = [0.0; 3];
    }
    Ok(preview)
}

/// A particle object has no permutations; the region list still needs a
/// selectable entry per region, so every object gets this single one.
const PARTICLE_OBJECT_PERMUTATION: &str = "default";

pub(in crate::app) fn build_render_preview(
    render_tag: &TagFile,
) -> Result<RenderModelPreview, String> {
    let render_model = RenderModel::from_tag(render_tag).map_err(|error| error.to_string())?;
    let render_meshes =
        RenderModel::derive_render_meshes(render_tag).map_err(|error| error.to_string())?;
    let mut preview = render_model_to_preview(&render_model, &render_meshes);
    append_model_errors(render_tag, &mut preview, ModelPreviewLayer::Render);
    Ok(preview)
}

/// Halo: Campaign Evolved model preview. Returns `None` when this isn't a CE
/// model (caller falls through to the classic render_model path); `Some(_)`
/// once we've recognized a CE `.model` (hlmt with a `skeleton model` ref
/// inside an IoStore container set), whether or not resolution succeeds.
pub(super) fn load_campaign_evolved_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    source: Option<&TagSource>,
    high_detail: bool,
) -> Option<Result<ModelPreviewData, String>> {
    let Some(src @ TagSource::IoStoreContainerSet { containers, .. }) = source else {
        return None;
    };
    if &model_tag.header.group_tag.to_be_bytes() != b"hlmt" {
        return None;
    }
    // Classic hlmt references a `render model`; CE references a `skeleton model`.
    let (_group, skel_ref) = model_tag.root().read_tag_ref_with_group("skeleton model")?;
    if skel_ref.trim().is_empty() {
        return None;
    }
    Some(build_campaign_evolved_preview(
        model_tag,
        entry,
        src,
        containers,
        &skel_ref,
        high_detail,
    ))
}

/// A Campaign Evolved model's skeleton and Unreal render geometry, resolved
/// the one way both its preview and its JMS export need.
struct CeModelMeshes {
    skel: TagFile,
    variants: Vec<ModelVariantPreview>,
    meshes: CeMeshes,
    /// Where the geometry came from, for the preview's label.
    render_path: String,
}

fn resolve_ce_model_meshes(
    model_tag: &TagFile,
    entry: &TagEntry,
    source: &TagSource,
    containers: &[MountedContainer],
    skel_ref: &str,
    high_detail: bool,
) -> Result<CeModelMeshes, String> {
    // 1. Resolve the skeleton_model (node skeleton + markers + regions) through
    //    the container tag index — the same read the browser tree performs.
    let skel = source
        .read_container_tag_by_ref(u32::from_be_bytes(*b"skel"), skel_ref)
        .map_err(|e| e.to_string())?;

    // 2. This model's package key, as DA_MeshSynchronization imports it
    //    (e.g. `objects/characters/elite_ai/elite_ai-model`).
    let TagEntryLocation::Container { rel_path, .. } = &entry.location else {
        return Err("A Campaign Evolved model needs a container entry.".to_owned());
    };
    let stem = rel_path.to_ascii_lowercase().replace('\\', "/");
    let stem = stem.strip_suffix(".ubulk").unwrap_or(&stem);
    let model_key = stem.rsplit("tags/").next().unwrap_or(stem).to_string();

    // 3. Read the hlmt variants (region→permutation): the set of
    //    (region, perm) pairs any variant activates.
    let variants = read_model_variants(model_tag);
    let mut needed: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();
    for v in &variants {
        for (region, perm) in &v.regions {
            if !perm.is_empty() {
                needed.insert((region.to_ascii_lowercase(), perm.to_ascii_lowercase()));
            }
        }
    }

    // 4. Authoritative path: the character/vehicle/weapon Blueprint bakes the
    //    exact region→perm→mesh binding into its mesh-sync RuntimeRegions —
    //    skeletal body + bone-attached static pieces. Fall back to the
    //    folder-scan heuristic (skeletal only) if that can't be found.
    let (meshes, render_path) = match ce_load_meshsync_regions(containers, &model_key) {
        Some(regions) => {
            // `high_detail` decodes the full-resolution Nanite geometry (slow,
            // millions of tris); otherwise the coarse LOD fallback (fast).
            (
                ce_collect_parts_from_regions(containers, &regions, &needed, high_detail),
                format!("meshsync:{model_key}"),
            )
        }
        None => (CeMeshes::default(), String::new()),
    };
    let (mut meshes, render_path) = if meshes.is_empty() {
        let char_root = ce_find_character_root(containers, &model_key).ok_or_else(|| {
            // First-person hand/body models (GameGlobals FirstPersonHands /
            // FirstPersonBody) are a separate representation: no world
            // DA_MeshSynchronization imports them, and their geometry is
            // sourced through the FirstPerson weapon/equipment actors — which
            // this world-model reconstruction doesn't rebuild.
            if is_first_person_model(&model_key) {
                "First-person hand/body models can't be rebuilt here — their geometry is \
                 provided by the first-person weapon actors, not the world mesh-sync path."
                    .to_owned()
            } else {
                "No MeshSynchronization data asset references this model — cannot locate its UE meshes."
                    .to_owned()
            }
        })?;
        let skeletal = ce_load_variant_meshes(containers, &char_root, &needed);
        (
            CeMeshes {
                skeletal,
                ..Default::default()
            },
            char_root,
        )
    } else {
        (meshes, render_path)
    };
    // Human characters' heads come from a separate MetaHuman `Face` component
    // (DT_MetaHumanHeads), not the mesh-sync path — resolve and fuse it in.
    let head_node = ce_head_node_name(&skel);
    ce_add_metahuman_head(
        containers,
        &model_key,
        &needed,
        &head_node,
        high_detail,
        &mut meshes,
    );
    if meshes.is_empty() {
        return Err("No UE meshes resolved for this model.".to_owned());
    }
    Ok(CeModelMeshes {
        skel,
        variants,
        meshes,
        render_path,
    })
}

/// The resolved meshes as the part lists the cross-game builders take.
fn ce_mesh_parts(
    meshes: &CeMeshes,
) -> (
    Vec<UeMeshPart<'_>>,
    Vec<UeStaticPart<'_>>,
    Vec<UeWorldPart<'_>>,
) {
    let parts = meshes
        .skeletal
        .iter()
        .map(|(region, perm, name, mesh, mats)| UeMeshPart {
            mesh: &**mesh,
            region: region.clone(),
            permutation: perm.clone(),
            name: name.clone(),
            material_names: mats.clone(),
        })
        .collect();
    let static_parts = meshes
        .statics
        .iter()
        .map(
            |(region, perm, name, mesh, bone, mats, xf, wa)| UeStaticPart {
                mesh: &**mesh,
                bone_name: bone.clone(),
                region: region.clone(),
                permutation: perm.clone(),
                name: name.clone(),
                material_names: mats.clone(),
                rel_transform: *xf,
                world_anchor: *wa,
            },
        )
        .collect();
    let world_parts = meshes
        .world
        .iter()
        .map(
            |(region, perm, name, mesh, node, mats, anchor)| UeWorldPart {
                mesh: &**mesh,
                node_name: node.clone(),
                head_anchor: *anchor,
                region: region.clone(),
                permutation: perm.clone(),
                name: name.clone(),
                material_names: mats.clone(),
            },
        )
        .collect();
    (parts, static_parts, world_parts)
}

fn build_campaign_evolved_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    source: &TagSource,
    containers: &[MountedContainer],
    skel_ref: &str,
    high_detail: bool,
) -> Result<ModelPreviewData, String> {
    let CeModelMeshes {
        skel,
        variants,
        meshes,
        render_path,
    } = resolve_ce_model_meshes(model_tag, entry, source, containers, skel_ref, high_detail)?;
    let (parts, static_parts, world_parts) = ce_mesh_parts(&meshes);

    // 5. Reconstruct the cross-game RenderModel and run the standard pipeline.
    let (render_model, render_meshes) =
        RenderModel::from_ue_meshes(&parts, &static_parts, &world_parts, &skel)
            .map_err(|e| e.to_string())?;
    if std::env::var("CE_DEBUG").is_ok() {
        let skel_nodes = skel
            .root()
            .field_path("nodes")
            .and_then(|f| f.as_block())
            .map(|b| b.len())
            .unwrap_or(0);
        eprintln!("[CE] skel_ref='{skel_ref}' skel_nodes={skel_nodes}");
        eprintln!(
            "[CE] skeletal parts: {}, static parts: {}",
            parts.len(),
            static_parts.len()
        );
        for (i, m) in render_meshes.iter().enumerate() {
            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
            for v in &m.vertices {
                let p = [v.position.x, v.position.y, v.position.z];
                for k in 0..3 {
                    mn[k] = mn[k].min(p[k]);
                    mx[k] = mx[k].max(p[k]);
                }
            }
            eprintln!(
                "[CE] mesh[{i}] {} verts center[{:.2} {:.2} {:.2}] extent[{:.2} {:.2} {:.2}] rigid_node={:?}",
                m.vertices.len(),
                (mn[0] + mx[0]) / 2.0,
                (mn[1] + mx[1]) / 2.0,
                (mn[2] + mx[2]) / 2.0,
                mx[0] - mn[0],
                mx[1] - mn[1],
                mx[2] - mn[2],
                m.rigid_node_index,
            );
        }
        for r in &render_model.regions {
            for p in &r.permutations {
                eprintln!(
                    "[CE] region '{}' perm '{}' idx={} count={}",
                    r.name, p.name, p.mesh_index, p.mesh_count
                );
            }
        }
    }
    let preview = render_model_to_preview(&render_model, &render_meshes);
    if preview.batches.is_empty() {
        return Err("Reconstructed CE model has no previewable geometry.".to_owned());
    }
    Ok(model_preview_data(
        entry.key.clone(),
        render_path,
        preview,
        variants,
    ))
}

/// Build a full-resolution JMS for a Campaign Evolved `hlmt` model by fusing
/// its Unreal render geometry (skeletal + **Nanite** static pieces) onto the
/// classic `skeleton_model` rig. Resolves the meshes as the preview does
/// ([`resolve_ce_model_meshes`]) but at full Nanite detail, and emits JMS —
/// the render-geometry half of model extraction (CE keeps render geometry in
/// Unreal, so there's no `render_model` tag to walk).
pub(in crate::app) fn campaign_evolved_render_jms(
    model_tag: &TagFile,
    entry: &TagEntry,
    source: &TagSource,
    skel_ref: &str,
) -> Result<blam_tags::jms::JmsFile, String> {
    let TagSource::IoStoreContainerSet { containers, .. } = source else {
        return Err("CE render extraction requires an IoStore container source.".to_owned());
    };
    let CeModelMeshes { skel, meshes, .. } =
        resolve_ce_model_meshes(model_tag, entry, source, containers, skel_ref, true)?;
    let (parts, static_parts, world_parts) = ce_mesh_parts(&meshes);
    let mut jms =
        blam_tags::jms::JmsFile::from_ue_meshes(&parts, &static_parts, &world_parts, &skel)
            .map_err(|e| e.to_string())?;
    apply_chimp_jms_uv_conventions(&mut jms, &meshes)?;
    Ok(jms)
}

/// The composite CE exporter must retain its classic skeleton retargeting and
/// region/permutation material cells. UV orientation and wrapping are safe to
/// take directly from Chimp's standalone converters; positions, normals and
/// weights remain assembly-specific because they must be retargeted to the tag
/// skeleton. The composite builder emits vertices in the same part order used
/// here.
fn apply_chimp_jms_uv_conventions(
    target: &mut blam_tags::jms::JmsFile,
    meshes: &CeMeshes,
) -> Result<(), String> {
    let mut vertex_offset = 0usize;

    for (_, _, _, mesh, materials) in &meshes.skeletal {
        let source = crate::app::chimp::chimp_skeletal_mesh_to_jms(mesh, materials);
        copy_chimp_jms_uvs(target, vertex_offset, &source)?;
        vertex_offset = vertex_offset.saturating_add(source.vertices.len());
    }
    for (_, _, _, mesh, _, materials, _, _) in &meshes.statics {
        let source = crate::app::chimp::chimp_static_mesh_to_jms(mesh, materials);
        copy_chimp_jms_uvs(target, vertex_offset, &source)?;
        vertex_offset = vertex_offset.saturating_add(source.vertices.len());
    }
    for (_, _, _, mesh, _, materials, _) in &meshes.world {
        let source = crate::app::chimp::chimp_skeletal_mesh_to_jms(mesh, materials);
        copy_chimp_jms_uvs(target, vertex_offset, &source)?;
        vertex_offset = vertex_offset.saturating_add(source.vertices.len());
    }

    if vertex_offset != target.vertices.len() {
        return Err(format!(
            "Chimp JMS vertex layout mismatch: converted {vertex_offset}, composite has {}",
            target.vertices.len()
        ));
    }
    Ok(())
}

fn copy_chimp_jms_uvs(
    target: &mut blam_tags::jms::JmsFile,
    vertex_offset: usize,
    source: &blam_tags::jms::JmsFile,
) -> Result<(), String> {
    let end = vertex_offset
        .checked_add(source.vertices.len())
        .ok_or_else(|| "Chimp JMS vertex range overflowed".to_owned())?;
    let target_vertices = target
        .vertices
        .get_mut(vertex_offset..end)
        .ok_or_else(|| "Chimp JMS vertex range does not match the composite model".to_owned())?;
    for (target, source) in target_vertices.iter_mut().zip(&source.vertices) {
        target.uvs.clone_from(&source.uvs);
    }
    Ok(())
}

/// Find a character's UE asset root by locating the `*MeshSynchronization`
/// data asset that imports this model's package, returning a
/// `.../characters/<name>` path prefix.
/// Heuristic: is this a first-person hand/body model (`spartans_fp`,
/// `.../fp_body`)? Those are `FirstPersonHands`/`FirstPersonBody`
/// representations sourced outside the world mesh-sync path.
fn is_first_person_model(model_key: &str) -> bool {
    let k = model_key.to_ascii_lowercase();
    k.contains("_fp/") || k.contains("/fp_") || k.contains("fp_body") || k.ends_with("_fp")
}

fn ce_find_character_root(containers: &[MountedContainer], model_key: &str) -> Option<String> {
    let index = ce_path_index(containers);
    for (norm, container, entry) in &index.mesh_sync {
        let c = &containers[*container];
        let e = &c.archive.entries()[*entry];
        let Ok(bytes) = c.archive.read(&e.path) else {
            continue;
        };
        let Ok(hdr) =
            FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
        else {
            continue;
        };
        let hit = hdr.imported_package_names.iter().any(|p| {
            p.to_ascii_lowercase()
                .replace('\\', "/")
                .ends_with(model_key)
        });
        if !hit {
            continue;
        }
        // Char folder = the dir holding this DA's `Common/` subfolder
        // (or the DA's own dir when it isn't under a `Common/`).
        return norm
            .rsplit_once("/common/")
            .map(|(root, _)| root.to_string())
            .or_else(|| norm.rsplit_once('/').map(|(root, _)| root.to_string()));
    }
    None
}

/// Native UClass object paths, identified by class (not filename) so resolution
/// is robust to the game's inconsistent asset-naming. A native class shows up in
/// an export's `class_index` as a `ScriptImport` whose value is the CityHash64
/// of the lowercased object path — so we compare against a precomputed
/// `FPackageObjectIndex` in O(1).
const CE_MESHSYNC_DA_CLASS: &str = "/Script/BlamSynchronization.BlamMeshSynchronizationDataAsset";
const CE_MESHSYNC_COMP_CLASS: &str = "/Script/BlamSynchronization.BlamMeshSynchronizationComponent";
const CE_MESHSYNC_COMP_BASE_CLASS: &str =
    "/Script/BlamSynchronization.BlamMeshSynchronizationComponentBase";

/// Precomputed class indices `(mesh-sync DA, component, component base)`.
static CE_CLASSES: LazyLock<(
    FPackageObjectIndex,
    FPackageObjectIndex,
    FPackageObjectIndex,
)> = LazyLock::new(|| {
    (
        FPackageObjectIndex::create_script_import(CE_MESHSYNC_DA_CLASS),
        FPackageObjectIndex::create_script_import(CE_MESHSYNC_COMP_CLASS),
        FPackageObjectIndex::create_script_import(CE_MESHSYNC_COMP_BASE_CLASS),
    )
});

/// One actor Blueprint that owns a mesh-sync component (identified by class).
#[derive(Clone)]
struct ActorRef {
    container: usize,
    path: String,
}

/// The mesh-sync binding graph for a mounted container set, built once and
/// cached: which actor Blueprints render which model. Built by a single
/// header-only pass (via [`IoStoreArchive::read_prefix`]) that identifies every
/// `BlamMeshSynchronizationDataAsset` and every actor with a
/// `BlamMeshSynchronizationComponent` **by class**, then joins them through the
/// import graph (actor → imports DA → DA's `ModelTag` is the model). No filename
/// heuristics: the class check catches `DA_*`, `BP_*`, and tokenless device
/// assets alike, and the import join is immune to codename aliases
/// (`tuning_fork`↔`Spirit`, `monitor`↔`GuiltySpark`, …).
struct CeMeshSyncIndex {
    /// `(imported model package name, lowercased) → actor Blueprints that render
    /// it`. Matched against a previewed model by suffix, preserving the exact
    /// `ends_with` semantics the resolver has always used.
    by_model: Vec<(String, Vec<ActorRef>)>,
}

/// Cache of the mesh-sync index, keyed by the mounted container set's identity
/// (its `.utoc` paths). Building it scans every package header once (~5s), so we
/// keep it for the life of the mount rather than rebuilding per preview.
static CE_INDEX_CACHE: LazyLock<Mutex<HashMap<String, Arc<CeMeshSyncIndex>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Every `.uasset` in a mounted container set, found by name without a scan.
struct CePathIndex {
    /// Lowercased file name (`sk_foo.uasset`) → `(container, entry)`, in the
    /// order a scan of containers then entries meets them, so the first match
    /// is the one a linear scan would have found.
    by_file_name: HashMap<String, Vec<(usize, usize)>>,
    /// The mesh-sync assets (lowercased, `/`-separated paths), in scan order.
    mesh_sync: Vec<(String, usize, usize)>,
}

/// Cache of [`CePathIndex`], keyed like [`CE_INDEX_CACHE`].
static CE_PATH_INDEX_CACHE: LazyLock<Mutex<HashMap<String, Arc<CePathIndex>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A container set's identity: its `.utoc` paths.
fn ce_container_set_key(containers: &[MountedContainer]) -> String {
    containers
        .iter()
        .map(|c| c.utoc_path.display().to_string())
        .collect::<Vec<_>>()
        .join("|")
}

fn build_ce_path_index(containers: &[MountedContainer]) -> CePathIndex {
    index_ce_paths(containers.iter().enumerate().flat_map(|(container, c)| {
        c.archive
            .entries()
            .iter()
            .enumerate()
            .map(move |(entry, e)| (container, entry, e.path.as_str()))
    }))
}

/// Index `(container, entry, path)` in the order given.
fn index_ce_paths<'p>(paths: impl Iterator<Item = (usize, usize, &'p str)>) -> CePathIndex {
    let mut by_file_name: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut mesh_sync = Vec::new();
    for (container, entry, path) in paths {
        let norm = path.to_ascii_lowercase().replace('\\', "/");
        if !norm.ends_with(".uasset") {
            continue;
        }
        let file_name = norm.rsplit('/').next().unwrap_or(&norm).to_owned();
        by_file_name
            .entry(file_name)
            .or_default()
            .push((container, entry));
        if norm.contains("meshsync") {
            mesh_sync.push((norm, container, entry));
        }
    }
    CePathIndex {
        by_file_name,
        mesh_sync,
    }
}

/// The cached path index for this container set (built on first use).
fn ce_path_index(containers: &[MountedContainer]) -> Arc<CePathIndex> {
    let key = ce_container_set_key(containers);
    let mut cache = CE_PATH_INDEX_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(key)
        .or_insert_with(|| Arc::new(build_ce_path_index(containers)))
        .clone()
}

/// Parse a package's Zen header cheaply — decode only the header prefix (name
/// map + import/export tables live at the front, before the bulky export data),
/// falling back to a full read for the rare header that exceeds the window.
fn ce_read_header(archive: &IoStoreArchive, path: &str) -> Option<FZenPackageHeader> {
    if let Ok(bytes) = archive.read_prefix(path, 192 * 1024) {
        if let Ok(hdr) =
            FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
        {
            return Some(hdr);
        }
    }
    let bytes = archive.read(path).ok()?;
    FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None).ok()
}

/// The lowercased basename stem (no dir, no extension) of a UE path.
fn ce_stem(path: &str) -> String {
    let n = path.to_ascii_lowercase().replace('\\', "/");
    let base = n.rsplit('/').next().unwrap_or(&n);
    base.strip_suffix(".uasset").unwrap_or(base).to_string()
}

/// Build the mesh-sync binding index for a container set (see [`CeMeshSyncIndex`]).
fn build_ce_mesh_sync_index(containers: &[MountedContainer]) -> CeMeshSyncIndex {
    let (da_class, comp_class, comp_base_class) = *CE_CLASSES;
    let t0 = std::time::Instant::now();

    // Single header-only pass over every `.uasset`.
    // `da_to_models`: DA stem → the `-model` package names it imports (its ModelTag).
    // `actors`: actor refs that own a mesh-sync component, with the DA stems they import.
    let mut da_to_models: HashMap<String, Vec<String>> = HashMap::new();
    let mut actors: Vec<(ActorRef, Vec<String>)> = Vec::new();
    let mut scanned = 0usize;
    for (ci, c) in containers.iter().enumerate() {
        for e in c.archive.entries() {
            if !e.path.to_ascii_lowercase().ends_with(".uasset") {
                continue;
            }
            let Some(hdr) = ce_read_header(&c.archive, &e.path) else {
                continue;
            };
            scanned += 1;
            let is_da = hdr.exports_class(da_class);
            let has_comp = hdr.exports_class(comp_class) || hdr.exports_class(comp_base_class);
            if !is_da && !has_comp {
                continue;
            }
            if is_da {
                let models: Vec<String> = hdr
                    .imported_package_names
                    .iter()
                    .map(|p| p.to_ascii_lowercase().replace('\\', "/"))
                    .filter(|p| p.ends_with("-model"))
                    .collect();
                if !models.is_empty() {
                    da_to_models
                        .entry(ce_stem(&e.path))
                        .or_default()
                        .extend(models);
                }
            }
            if has_comp {
                let imported_stems: Vec<String> = hdr
                    .imported_package_names
                    .iter()
                    .map(|p| ce_stem(p))
                    .collect();
                actors.push((
                    ActorRef {
                        container: ci,
                        path: e.path.clone(),
                    },
                    imported_stems,
                ));
            }
        }
    }

    // Join: an actor renders model M if it imports a DA whose ModelTag is M.
    let mut model_to_actors: HashMap<String, Vec<ActorRef>> = HashMap::new();
    for (actor, imported_stems) in &actors {
        let mut models: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for stem in imported_stems {
            if let Some(m) = da_to_models.get(stem) {
                models.extend(m.iter().cloned());
            }
        }
        for m in models {
            let list = model_to_actors.entry(m).or_default();
            if !list
                .iter()
                .any(|a| a.container == actor.container && a.path == actor.path)
            {
                list.push(actor.clone());
            }
        }
    }

    let by_model: Vec<(String, Vec<ActorRef>)> = model_to_actors.into_iter().collect();
    if std::env::var("CE_DEBUG").is_ok() {
        eprintln!(
            "[CE] mesh-sync index: {} DAs, {} component-actors, {} models, from {scanned} headers in {:.1}s",
            da_to_models.len(),
            actors.len(),
            by_model.len(),
            t0.elapsed().as_secs_f32(),
        );
    }
    CeMeshSyncIndex { by_model }
}

/// The cached mesh-sync index for this container set (built on first use).
fn ce_mesh_sync_index(containers: &[MountedContainer]) -> Arc<CeMeshSyncIndex> {
    let key = ce_container_set_key(containers);
    // Held for the whole build, so a preview that asks while the prewarm is
    // running waits for it instead of scanning every header a second time. A
    // build that panicked poisons the lock; the map it guards is still sound.
    let mut cache = CE_INDEX_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(idx) = cache.get(&key) {
        return idx.clone();
    }
    let idx = Arc::new(build_ce_mesh_sync_index(containers));
    cache.insert(key, idx.clone());
    idx
}

/// Build a Campaign Evolved container set's mesh-sync index in the background
/// as soon as it is mounted.
///
/// The first model preview used to build it on the UI thread, which scans
/// every package header in the install: about five seconds with the app
/// frozen. Built here, the first preview finds it ready (or waits on this
/// build rather than starting its own).
pub(in crate::app) fn prewarm_ce_mesh_sync_index(containers: Vec<MountedContainer>) {
    spawn_background("Campaign Evolved mesh index prewarm", move || {
        ce_path_index(&containers);
        ce_mesh_sync_index(&containers);
    });
}

// ---------------------------------------------------------------------------
// MetaHuman head resolution
//
// Human characters source their head from a separate MetaHuman `Face` component
// driven at runtime by `BPC_MetaHumanCreator` + the `DT_MetaHumanHeads` data
// table — NOT the mesh-sync `RuntimeRegions`. So a human's head never appears in
// the mesh-sync path and must be resolved here: character key (from the actor
// Blueprint's name) → DT row → face + facial-hair skeletal meshes, baked onto
// the classic `head` node. The DataTable's Blueprint row struct is absent from
// the native `.usmap`, so its layout is first recovered from the
// `UUserDefinedStruct` export and registered before decoding.
// ---------------------------------------------------------------------------

/// The Blueprint component class that marks an actor as having a MetaHuman head.
const CE_MH_COMPONENT: &str = "BPC_MetaHumanCreator";

/// A decoded `DT_MetaHumanHeads` row: the face mesh + optional facial-hair
/// meshes, each as a `(package, asset)` soft reference.
#[derive(Clone, Default)]
struct MetaHumanHeadRow {
    head: Option<(String, String)>,
    hair: Vec<(String, String)>,
    /// The `DT_MetaHumanHeads` `Type` field — `unique` (heroes), `male`, or
    /// `female`. The game groups rows by this (`CreateHeadArrayGroups_*`): heroes
    /// are looked up by name; generics randomize within their gender's group.
    type_: String,
}

/// A decoded `DT_MetaHumanHelmets` row: the helmet/hat mesh `(package, asset)`.
/// Some helmets are static (`SM_*`, head-bone-local), others skeletal (`SK_*`,
/// world-space) — flagged so each takes the right bake path.
#[derive(Clone, Default)]
struct MetaHumanHelmetRow {
    mesh: Option<(String, String)>,
    skeletal: bool,
}

/// The decoded MetaHuman head + helmet tables for a mounted container set (row
/// key, lowercased → meshes).
struct CeMetaHumanTables {
    heads: HashMap<String, MetaHumanHeadRow>,
    helmets: HashMap<String, MetaHumanHelmetRow>,
}

static CE_MH_CACHE: LazyLock<Mutex<HashMap<String, Arc<CeMetaHumanTables>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn ce_metahuman_tables(containers: &[MountedContainer]) -> Arc<CeMetaHumanTables> {
    let key = containers
        .iter()
        .map(|c| c.utoc_path.display().to_string())
        .collect::<Vec<_>>()
        .join("|");
    let mut cache = CE_MH_CACHE.lock().unwrap();
    if let Some(t) = cache.get(&key) {
        return t.clone();
    }
    fn lower<T>(v: Vec<(String, T)>) -> HashMap<String, T> {
        v.into_iter().map(|(k, r)| (k.to_lowercase(), r)).collect()
    }
    let t = Arc::new(CeMetaHumanTables {
        heads: lower(ce_decode_head_table(containers).unwrap_or_default()),
        helmets: lower(ce_decode_helmet_table(containers).unwrap_or_default()),
    });
    cache.insert(key, t.clone());
    t
}

/// Find a `.uasset` by its exact basename (no extension), returning `(container
/// index, bytes)`. Used for the singleton MetaHuman data tables / row structs.
fn ce_find_uasset_by_basename(containers: &[MountedContainer], basename: &str) -> Option<Vec<u8>> {
    let index = ce_path_index(containers);
    let candidates = index
        .by_file_name
        .get(&format!("{}.uasset", basename.to_ascii_lowercase()))?;
    candidates.iter().find_map(|&(container, entry)| {
        let c = &containers[container];
        c.archive.read(&c.archive.entries()[entry].path).ok()
    })
}

/// Export[0]'s serial byte slice within a package.
fn ce_export0_bytes<'a>(bytes: &'a [u8], hdr: &FZenPackageHeader) -> Option<&'a [u8]> {
    let ex = hdr.export_map.first()?;
    let start = hdr.summary.header_size as usize + ex.cooked_serial_offset as usize;
    bytes.get(start..start + ex.cooked_serial_size as usize)
}

/// Recover a Blueprint DataTable's row-struct layout from its
/// `UUserDefinedStruct` asset, register it, and decode the table's rows. The
/// `.usmap` is native-reflection only, so the row struct must be recovered
/// first. Shared by the head and helmet tables.
fn ce_decode_metahuman_table(
    containers: &[MountedContainer],
    struct_basename: &str,
    struct_name: &str,
    table_basename: &str,
) -> Option<Vec<(String, PropertyBlock)>> {
    let mut usmap = Usmap::meteorite().ok()?;
    let sbytes = ce_find_uasset_by_basename(containers, struct_basename)?;
    let shdr =
        FZenPackageHeader::deserialize(&mut Cursor::new(&sbytes[..]), None, CE_CV, CE_HV, None)
            .ok()?;
    let sctx = blam_tags::iostore::unversioned::ExportContext::new(&[]);
    let props = blam_tags::iostore::unversioned::read_userdefined_struct_layout(
        ce_export0_bytes(&sbytes, &shdr)?,
        &shdr.name_map.copy_raw_names(),
        &usmap,
        shdr.export_map.first()?.object_flags,
        &sctx,
    )
    .ok()?;
    usmap.register_struct(struct_name, None, props);
    let dbytes = ce_find_uasset_by_basename(containers, table_basename)?;
    let dhdr =
        FZenPackageHeader::deserialize(&mut Cursor::new(&dbytes[..]), None, CE_CV, CE_HV, None)
            .ok()?;
    blam_tags::iostore::unversioned::read_datatable(
        ce_export0_bytes(&dbytes, &dhdr)?,
        &dhdr.name_map.copy_raw_names(),
        &usmap,
        struct_name,
        dhdr.export_map.first()?.object_flags,
    )
    .ok()
}

/// A non-empty soft-object path as `(package, asset)`.
fn ce_soft(sp: &blam_tags::iostore::unversioned::SoftObjectPath) -> Option<(String, String)> {
    (!sp.is_empty()).then(|| {
        (
            sp.package.as_str().to_string(),
            sp.asset.as_str().to_string(),
        )
    })
}

/// Decode `DT_MetaHumanHeads` → per-row face + facial-hair mesh references.
fn ce_decode_head_table(
    containers: &[MountedContainer],
) -> Option<Vec<(String, MetaHumanHeadRow)>> {
    let rows = ce_decode_metahuman_table(
        containers,
        "S_MetaHumanHeads",
        "S_MetaHumanHeads",
        "DT_MetaHumanHeads",
    )?;
    Some(
        rows.into_iter()
            .map(|(key, fields)| {
                let mut row = MetaHumanHeadRow::default();
                if let Some(sp) = fields.get("Head").and_then(PropValue::as_soft_object) {
                    row.head = ce_soft(sp);
                }
                if let Some(arr) = fields.get("FacialHair").and_then(PropValue::as_array) {
                    row.hair = arr
                        .iter()
                        .filter_map(PropValue::as_soft_object)
                        .filter_map(ce_soft)
                        .collect();
                }
                row.type_ = fields
                    .get("Type")
                    .and_then(PropValue::as_str)
                    .unwrap_or_default()
                    .to_string();
                (key, row)
            })
            .collect(),
    )
}

/// Decode `DT_MetaHumanHelmets` → per-row helmet/hat mesh reference.
fn ce_decode_helmet_table(
    containers: &[MountedContainer],
) -> Option<Vec<(String, MetaHumanHelmetRow)>> {
    let rows = ce_decode_metahuman_table(
        containers,
        "S_MetaHumanHelmets",
        "S_MetaHumanHelmets",
        "DT_MetaHumanHelmets",
    )?;
    Some(
        rows.into_iter()
            .map(|(key, fields)| {
                let mut row = MetaHumanHelmetRow::default();
                if let Some(m) = fields
                    .get("Mesh")
                    .and_then(PropValue::as_soft_object)
                    .and_then(ce_soft)
                {
                    row.skeletal = m.1.to_ascii_lowercase().starts_with("sk_");
                    row.mesh = Some(m);
                }
                (key, row)
            })
            .collect(),
    )
}

/// The MetaHuman character key for a model (the `DT_MetaHumanHeads` row key),
/// derived from its actor Blueprint's name (`BP_JohnsonBipedActor` → `johnson`),
/// but only when that actor actually mounts a MetaHuman head component
/// (`BPC_MetaHumanCreator`). Non-human actors return `None`.
fn ce_metahuman_character_key(containers: &[MountedContainer], model_key: &str) -> Option<String> {
    let index = ce_mesh_sync_index(containers);
    let refs: Vec<&ActorRef> = index
        .by_model
        .iter()
        .filter(|(m, _)| m.ends_with(model_key))
        .flat_map(|(_, a)| a.iter())
        .collect();
    for r in refs {
        let c = containers.get(r.container)?;
        let Some(hdr) = ce_read_header(&c.archive, &r.path) else {
            continue;
        };
        let is_human = hdr.imported_package_names.iter().any(|p| {
            p.rsplit('/')
                .next()
                .unwrap_or(p)
                .eq_ignore_ascii_case(CE_MH_COMPONENT)
        });
        if !is_human {
            continue;
        }
        let base = r.path.rsplit('/').next().unwrap_or(&r.path);
        let base = base
            .strip_suffix(".uasset")
            .unwrap_or(base)
            .to_ascii_lowercase();
        // `bp_<key>bipedactor` → `<key>`.
        let key = base.strip_prefix("bp_").unwrap_or(&base);
        let key = key.strip_suffix("bipedactor").unwrap_or(key);
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }
    None
}

/// Resolve and append a human model's MetaHuman head (face + facial hair) to
/// `out`, bound to the classic `head` node for every needed head-region
/// permutation. A no-op for non-human models (no MetaHuman actor / no matching
/// DT row).
/// The skeleton's head node name — CE human rigs use `head_m` (midline `_m`
/// suffix), but resolve it from the skeleton so a differently-named rig still
/// binds. Falls back to `head_m`.
fn ce_head_node_name(skel: &TagFile) -> String {
    let root = skel.root();
    if let Some(block) = root.field_path("nodes").and_then(|f| f.as_block()) {
        let names: Vec<String> = (0..block.len())
            .filter_map(|i| block.element(i).and_then(|e| e.read_string_id("name")))
            .collect();
        if let Some(n) = names
            .iter()
            .find(|n| n.eq_ignore_ascii_case("head_m") || n.eq_ignore_ascii_case("head"))
        {
            return n.clone();
        }
        if let Some(n) = names
            .iter()
            .find(|n| n.to_ascii_lowercase().contains("head"))
        {
            return n.clone();
        }
    }
    "head_m".to_string()
}

fn ce_add_metahuman_head(
    containers: &[MountedContainer],
    model_key: &str,
    needed: &std::collections::BTreeSet<(String, String)>,
    head_node: &str,
    high_detail: bool,
    out: &mut CeMeshes,
) {
    let Some(key) = ce_metahuman_character_key(containers, model_key) else {
        return;
    };
    let tables = ce_metahuman_tables(containers);

    // The model's permutations for `region` (fall back to every distinct
    // permutation it needs when the region declares none).
    let perms_for = |region: &str| -> Vec<String> {
        let mut p: Vec<String> = needed
            .iter()
            .filter(|(r, _)| r.eq_ignore_ascii_case(region))
            .map(|(_, p)| p.clone())
            .collect();
        if p.is_empty() {
            p = needed.iter().map(|(_, p)| p.clone()).collect();
            p.sort();
            p.dedup();
        }
        p
    };

    // Captured from the face rig while emitting the head; anchors the hat.
    let mut head_anchor: Option<[f32; 3]> = None;

    // Head: the face + facial-hair skeletal meshes, world-space baked to `head`.
    // Mirrors the game's `BPC_MetaHumanCreator`: heroes (johnson/keyes/…) key
    // straight to a `DT_MetaHumanHeads` row (its `GetDataTableRowFromName`);
    // generic humans (crewman/marine, keyed malecrewman/basemarine/…) have no
    // per-character row — the game randomizes within the gender's `Type` group
    // (`CreateHeadArrayGroups_{Male,Female}_Names`). There is no canonical head
    // for a generic, so for a stable preview we pick the first row of the matching
    // `Type` (grouping by the authoritative DT `Type` field, not the row name).
    let head_row = tables.heads.get(&key).or_else(|| {
        // Gender from the character key or the model path (`.../marine_female/...`
        // or `crewman_female`) — the game's `bIsFemale`/`bUseFemaleHeads`. Checking
        // both survives the non-deterministic actor→key pick when several actors
        // share a model. Default male.
        let female = key.contains("female") || model_key.to_ascii_lowercase().contains("female");
        let want_type = if female { "female" } else { "male" };
        tables
            .heads
            .iter()
            .filter(|(_, row)| row.type_.eq_ignore_ascii_case(want_type))
            .min_by(|a, b| a.0.cmp(b.0))
            .map(|(_, row)| row)
    });
    if let Some(row) = head_row {
        let head_perms = perms_for("head");
        let mut refs: Vec<&(String, String)> = Vec::new();
        if let Some(h) = &row.head {
            refs.push(h);
        }
        refs.extend(row.hair.iter());
        for (i, (pkg, asset)) in refs.iter().enumerate() {
            let Some(mesh) = ce_read_skeletal_mesh(containers, pkg) else {
                continue;
            };
            // Each mesh's own `head` bone world position anchors it to the classic
            // head node; the face's (first ref) also marks the hat below.
            let anchor = ce_metahuman_head_anchor(&mesh).unwrap_or([0.0; 3]);
            if i == 0 {
                head_anchor = Some(anchor);
            }
            let mats = ce_read_default_materials(containers, pkg);
            for perm in &head_perms {
                out.world.push((
                    "head".to_string(),
                    perm.clone(),
                    asset.clone(),
                    mesh.clone(),
                    head_node.to_string(),
                    mats.clone(),
                    anchor,
                ));
            }
        }
    }

    // Helmet/hat: `SM_*` are hats authored world-aligned at the MetaHuman head
    // socket (anchored to the face rig's `head` bone); `SK_*` are world-space
    // skeletal helmets. Emit to the `helmet` region on the classic head node.
    if let Some(hrow) = tables.helmets.get(&key) {
        if let Some((pkg, asset)) = &hrow.mesh {
            let helmet_perms = perms_for("helmet");
            if hrow.skeletal {
                if let Some(mesh) = ce_read_skeletal_mesh(containers, pkg) {
                    let mats = ce_read_default_materials(containers, pkg);
                    let anchor = ce_metahuman_head_anchor(&mesh).unwrap_or([0.0; 3]);
                    for perm in &helmet_perms {
                        out.world.push((
                            "helmet".to_string(),
                            perm.clone(),
                            asset.clone(),
                            mesh.clone(),
                            head_node.to_string(),
                            mats.clone(),
                            anchor,
                        ));
                    }
                }
            } else if let Some(mesh) = ce_read_static_mesh(containers, pkg, high_detail) {
                let mats = ce_read_default_materials(containers, pkg);
                for perm in &helmet_perms {
                    out.statics.push((
                        "helmet".to_string(),
                        perm.clone(),
                        asset.clone(),
                        mesh.clone(),
                        head_node.to_string(),
                        mats.clone(),
                        blam_tags::iostore::unversioned::MeshTransform::default(),
                        head_anchor,
                    ));
                }
            }
        }
    }
}

/// The MetaHuman face rig's `head` bone world position (UE cm) — the anchor a
/// hat/helmet is authored relative to. `None` if the mesh has no head bone.
fn ce_metahuman_head_anchor(face: &SkeletalMesh) -> Option<[f32; 3]> {
    let idx = face
        .bones
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case("head"))
        .or_else(|| {
            face.bones
                .iter()
                .position(|b| b.name.to_ascii_lowercase().contains("head"))
        })?;
    let world = blam_tags::jms::ue_bind_world(&face.bones);
    let m = world.get(idx)?;
    Some([m.m[0][3], m.m[1][3], m.m[2][3]])
}

/// Read + decode a `USkeletalMesh` by package path (`None` on any failure).
fn ce_read_skeletal_mesh(containers: &[MountedContainer], pkg: &str) -> Option<Arc<SkeletalMesh>> {
    let (_, bytes) = ce_read_uasset_by_package(containers, pkg)?;
    let hdr =
        FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
            .ok()?;
    let names = hdr.name_map.copy_raw_names();
    SkeletalMesh::from_package(&bytes, &names, hdr.summary.header_size as usize)
        .ok()
        .map(Arc::new)
}

/// Read + decode a `UStaticMesh` by package, preferring Nanite bulk data when
/// `high_detail` is enabled and falling back to the coarse cooked LOD.
fn ce_read_static_mesh(
    containers: &[MountedContainer],
    pkg: &str,
    high_detail: bool,
) -> Option<Arc<StaticMesh>> {
    let (_, bytes) = ce_read_uasset_by_package(containers, pkg)?;
    let hdr =
        FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
            .ok()?;
    let bulk = high_detail
        .then(|| ce_read_bulk_by_package(containers, pkg))
        .flatten();
    StaticMesh::from_package_preferring_nanite(
        &bytes,
        hdr.summary.header_size as usize,
        bulk.as_deref(),
    )
    .ok()
    .map(Arc::new)
}

/// The default per-slot materials of a mesh package (its `MI_`/`M_` imports).
fn ce_read_default_materials(containers: &[MountedContainer], pkg: &str) -> Vec<String> {
    let Some((_, bytes)) = ce_read_uasset_by_package(containers, pkg) else {
        return Vec::new();
    };
    let Ok(hdr) =
        FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
    else {
        return Vec::new();
    };
    ce_default_materials(&hdr)
}

/// Decode the authoritative region→permutation→mesh mapping for a model from
/// the actor Blueprint(s) that render it. Chain: this model's package is
/// imported by a `BlamMeshSynchronizationDataAsset` (identified by class), which
/// is imported by the actor Blueprint whose `BlamMeshSynchronizationComponent`
/// (also by class) bakes the `RuntimeRegions` map. Actors and their bindings are
/// resolved once into a cached [`CeMeshSyncIndex`]; this just decodes the few
/// relevant actors' world regions and merges them.
fn ce_load_meshsync_regions(
    containers: &[MountedContainer],
    model_key: &str,
) -> Option<MeshSyncRegions> {
    let index = ce_mesh_sync_index(containers);
    // The actors whose DA's ModelTag is this model (suffix match, preserving the
    // resolver's long-standing `ends_with` semantics). Several DAs can name the
    // same model (a world biped's DA plus the first-person arms/legs DAs), so
    // gather all their actors and let the `is_world` filter below select.
    let mut refs: Vec<&ActorRef> = index
        .by_model
        .iter()
        .filter(|(m, _)| m.ends_with(model_key))
        .flat_map(|(_, a)| a.iter())
        .collect();
    refs.sort_by(|a, b| (a.container, &a.path).cmp(&(b.container, &b.path)));
    refs.dedup_by(|a, b| a.container == b.container && a.path == b.path);
    if refs.is_empty() {
        return None;
    }

    let (_, comp_class, comp_base_class) = *CE_CLASSES;
    let usmap = Usmap::meteorite().ok()?;
    let mut merged = MeshSyncRegions::default();
    for r in refs {
        let Some(c) = containers.get(r.container) else {
            continue;
        };
        let Ok(bytes) = c.archive.read(&r.path) else {
            continue;
        };
        let Ok(hdr) =
            FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
        else {
            continue;
        };
        // The mesh-sync component export, by class.
        let Some(comp) = hdr
            .find_export_of_class(comp_class)
            .or_else(|| hdr.find_export_of_class(comp_base_class))
        else {
            continue;
        };
        let start = hdr.summary.header_size as usize + comp.cooked_serial_offset as usize;
        let end = start + comp.cooked_serial_size as usize;
        let Some(export) = bytes.get(start..end) else {
            continue;
        };
        let names = hdr.name_map.copy_raw_names();
        if let Ok(regions) = MeshSyncRegions::from_component_export(export, &names, &usmap) {
            // Only world representation — the first-person pawn's arms/legs
            // components (`is_world() == false`) also reference the same model
            // and must not leak into the world preview.
            if regions.is_world() {
                ce_merge_regions(&mut merged, regions);
            }
        }
    }
    (!merged.regions.is_empty()).then_some(merged)
}

/// Merge one actor's world `RuntimeRegions` into the accumulator, deduping
/// meshes by `(package, parent_bone)` so overlapping actors (or world variants
/// of the same actor) don't double-emit the same piece.
fn ce_merge_regions(dst: &mut MeshSyncRegions, src: MeshSyncRegions) {
    fn add(dst: &mut Vec<MeshRef>, m: MeshRef) {
        if !dst.iter().any(|x| {
            x.package.eq_ignore_ascii_case(&m.package)
                && x.parent_bone.eq_ignore_ascii_case(&m.parent_bone)
        }) {
            dst.push(m);
        }
    }
    for sr in src.regions {
        let ri = match dst
            .regions
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(&sr.name))
        {
            Some(i) => i,
            None => {
                dst.regions.push(Region {
                    name: sr.name.clone(),
                    permutations: Vec::new(),
                });
                dst.regions.len() - 1
            }
        };
        for sp in sr.permutations {
            let perms = &mut dst.regions[ri].permutations;
            let pi = match perms
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(&sp.name))
            {
                Some(i) => i,
                None => {
                    perms.push(Permutation {
                        name: sp.name.clone(),
                        skeletal_meshes: Vec::new(),
                        static_meshes: Vec::new(),
                    });
                    perms.len() - 1
                }
            };
            for m in sp.skeletal_meshes {
                add(&mut perms[pi].skeletal_meshes, m);
            }
            for m in sp.static_meshes {
                add(&mut perms[pi].static_meshes, m);
            }
        }
    }
}

/// A model's resolved UE geometry: skinned skeletal meshes + rigid static
/// pieces (each attached to a bone), tagged with their authoritative
/// `(region, permutation)`.
#[derive(Default)]
struct CeMeshes {
    /// `(region, perm, asset name, mesh, material names)`. Meshes are shared
    /// (`Arc`) because color variants reference the same geometry — loading and
    /// (Nanite-)decoding each package once, not once per variant.
    skeletal: Vec<(
        String,
        String,
        String,
        std::sync::Arc<SkeletalMesh>,
        Vec<String>,
    )>,
    /// `(region, perm, asset name, mesh, parent bone, material names, xform,
    /// world_anchor)`. `world_anchor = Some(pos)` marks a MetaHuman hat baked
    /// world-aligned at the face rig's head-bone position `pos` (UE cm), vs.
    /// `None` for a mesh-sync vehicle part in its bone's local frame.
    statics: Vec<(
        String,
        String,
        String,
        std::sync::Arc<StaticMesh>,
        String,
        Vec<String>,
        blam_tags::iostore::unversioned::MeshTransform,
        Option<[f32; 3]>,
    )>,
    /// `(region, perm, asset name, mesh, target node, material names,
    /// head_anchor)` — MetaHuman `Face`/hair meshes on a foreign rig, placed with
    /// their `head` bone (`head_anchor`, UE cm) at `node`'s classic position (see
    /// [`UeWorldPart`]). Sourced from `DT_MetaHumanHeads`, not the mesh-sync
    /// `RuntimeRegions`.
    world: Vec<(
        String,
        String,
        String,
        std::sync::Arc<SkeletalMesh>,
        String,
        Vec<String>,
        [f32; 3],
    )>,
}

impl CeMeshes {
    fn is_empty(&self) -> bool {
        self.skeletal.is_empty() && self.statics.is_empty() && self.world.is_empty()
    }
}

/// Strip the material-instance/material asset prefix (`MI_`/`MIP_`/`M_`) so the
/// emitted JMS material name is the clean shader name a `tool.exe` shader tag
/// binds to.
fn strip_material_prefix(name: &str) -> String {
    for p in ["MIP_", "MI_", "M_"] {
        if let Some(rest) = name.strip_prefix(p) {
            return rest.to_string();
        }
    }
    name.to_string()
}

/// A mesh's default per-slot materials: the material-instance packages it
/// imports (`MI_`/`M_`), in import order — the order a section's
/// `material_index` addresses. UE names each material slot after its default
/// material, so an instance's [`MeshRef::material_overrides`] key (a slot name)
/// matches one of these entries by name.
fn ce_default_materials(hdr: &FZenPackageHeader) -> Vec<String> {
    hdr.imported_package_names
        .iter()
        .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
        .filter(|b| b.starts_with("MI_") || b.starts_with("M_"))
        .collect()
}

/// The effective per-slot material names for one mesh instance: the mesh's
/// default slot materials with this instance's variant overrides applied. An
/// override binds by slot name, which equals the default material's name, so we
/// replace the matching default in place (exact, order-preserving) — no reliance
/// on decoding the mesh's slot array. Names are prefix-stripped for tool.exe.
fn ce_effective_materials(
    default_materials: &[String],
    overrides: &[(String, String)],
) -> Vec<String> {
    let mut mats = default_materials.to_vec();
    for (slot, over) in overrides {
        if let Some(pos) = mats.iter().position(|m| m.eq_ignore_ascii_case(slot)) {
            mats[pos] = over.clone();
        }
        // A slot name that matches no default import means a renamed slot we
        // can't position without the mesh slot array; skip rather than misalign
        // the `material_index → name` mapping.
    }
    mats.iter().map(|m| strip_material_prefix(m)).collect()
}

/// Load the UE meshes the authoritative mapping binds to each needed
/// `(region, permutation)` — skinned `SkeletalMesh`es plus rigid `StaticMesh`
/// pieces (each with its `parent_bone`), including multi-mesh permutations
/// (e.g. arms = anatomy skin + sleeve, or a vehicle body + dozens of parts).
fn ce_collect_parts_from_regions(
    containers: &[MountedContainer],
    regions: &MeshSyncRegions,
    needed: &std::collections::BTreeSet<(String, String)>,
    nanite: bool,
) -> CeMeshes {
    let mut out = CeMeshes::default();
    // Cache loaded+decoded meshes by package, so a mesh referenced by dozens of
    // color-variant permutations is read and (Nanite-)decoded exactly once.
    // `None` marks a package that failed to load (don't retry it per variant).
    let mut sk_cache: std::collections::HashMap<
        String,
        Option<(std::sync::Arc<SkeletalMesh>, Vec<String>)>,
    > = std::collections::HashMap::new();
    let mut sm_cache: std::collections::HashMap<
        String,
        Option<(std::sync::Arc<StaticMesh>, Vec<String>)>,
    > = std::collections::HashMap::new();
    for (region, perm) in needed {
        for mref in regions.skeletal_meshes(region, perm) {
            let entry = sk_cache.entry(mref.package.clone()).or_insert_with(|| {
                let (_, bytes) = ce_read_uasset_by_package(containers, &mref.package)?;
                let hdr = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&bytes[..]),
                    None,
                    CE_CV,
                    CE_HV,
                    None,
                )
                .ok()?;
                let names = hdr.name_map.copy_raw_names();
                let mesh =
                    SkeletalMesh::from_package(&bytes, &names, hdr.summary.header_size as usize)
                        .ok()?;
                Some((std::sync::Arc::new(mesh), ce_default_materials(&hdr)))
            });
            if let Some((mesh, default_mats)) = entry {
                out.skeletal.push((
                    region.clone(),
                    perm.clone(),
                    mref.asset.clone(),
                    mesh.clone(),
                    ce_effective_materials(default_mats, &mref.material_overrides),
                ));
            }
        }
        for mref in regions.static_meshes(region, perm) {
            let entry = sm_cache.entry(mref.package.clone()).or_insert_with(|| {
                let (_, bytes) = ce_read_uasset_by_package(containers, &mref.package)?;
                let hdr = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&bytes[..]),
                    None,
                    CE_CV,
                    CE_HV,
                    None,
                )
                .ok()?;
                // For extraction (and the preview's high-detail mode) prefer the
                // full-resolution Nanite geometry from the package's `.ubulk`;
                // the light preview uses the coarse LOD fallback.
                let ubulk = if nanite {
                    ce_read_bulk_by_package(containers, &mref.package)
                } else {
                    None
                };
                let mesh = StaticMesh::from_package_preferring_nanite(
                    &bytes,
                    hdr.summary.header_size as usize,
                    ubulk.as_deref(),
                )
                .ok()?;
                Some((std::sync::Arc::new(mesh), ce_default_materials(&hdr)))
            });
            if let Some((mesh, default_mats)) = entry {
                out.statics.push((
                    region.clone(),
                    perm.clone(),
                    mref.asset.clone(),
                    mesh.clone(),
                    mref.parent_bone.clone(),
                    ce_effective_materials(default_mats, &mref.material_overrides),
                    mref.rel_transform,
                    None,
                ));
            }
        }
    }
    if std::env::var("CE_DEBUG").is_ok() {
        let sk_ok = sk_cache.values().filter(|v| v.is_some()).count();
        let sm_ok = sm_cache.values().filter(|v| v.is_some()).count();
        eprintln!(
            "[CE] cache: {sk_ok} unique SK decoded (of {} sk parts), {sm_ok} unique SM decoded (of {} sm parts)",
            out.skeletal.len(),
            out.statics.len()
        );
    }
    out
}

/// The `.uasset` entries of a UE package path (`/Game/Characters/.../SK_Foo`):
/// those whose path ends with the corresponding `/...SK_Foo.uasset` tail, in
/// the order a scan of every container meets them.
///
/// Answered from the file-name index: a path with that tail has the tail's
/// last segment as its file name, so only those few entries are compared.
/// This used to lowercase and rewrite every path in the install, several
/// times per mesh.
fn ce_package_entries<'c>(
    containers: &'c [MountedContainer],
    package: &str,
) -> impl Iterator<Item = (&'c MountedContainer, &'c blam_tags::iostore::Entry)> {
    let (file_name, suffix) = ce_package_file_name_and_suffix(package);
    let candidates = ce_path_index(containers)
        .by_file_name
        .get(&file_name)
        .cloned()
        .unwrap_or_default();
    candidates
        .into_iter()
        .filter_map(move |(container, entry)| {
            let c = &containers[container];
            let e = &c.archive.entries()[entry];
            e.path
                .to_ascii_lowercase()
                .replace('\\', "/")
                .ends_with(&suffix)
                .then_some((c, e))
        })
}

/// A package path's `.uasset` file name, and the path tail a matching entry
/// ends with: `/Game/A/SK_Foo` → (`sk_foo.uasset`, `/a/sk_foo.uasset`).
fn ce_package_file_name_and_suffix(package: &str) -> (String, String) {
    let tail = package.to_ascii_lowercase().replace('\\', "/");
    let tail = tail.strip_prefix("/game/").unwrap_or(&tail);
    let file_name = format!("{}.uasset", tail.rsplit('/').next().unwrap_or(tail));
    (file_name, format!("/{tail}.uasset"))
}

/// Read a `.uasset` by its UE package path; see [`ce_package_entries`].
fn ce_read_uasset_by_package(
    containers: &[MountedContainer],
    package: &str,
) -> Option<(String, Vec<u8>)> {
    ce_package_entries(containers, package).find_map(|(c, e)| {
        c.archive
            .read(&e.path)
            .ok()
            .map(|bytes| (e.path.clone(), bytes))
    })
}

/// Read the sibling `.ubulk` (Nanite streaming pages) for a package, matched
/// the same way as [`ce_read_uasset_by_package`]. Bulk data isn't in the
/// directory index — it shares the package's chunk id with the BulkData type,
/// fetched via [`IoStoreArchive::read_bulk_for`].
fn ce_read_bulk_by_package(containers: &[MountedContainer], package: &str) -> Option<Vec<u8>> {
    let (c, e) = ce_package_entries(containers, package).next()?;
    c.archive.read_bulk_for(e.chunk_index, 0).ok()
}

/// Variant-driven mesh loading: for each `(region, permutation)` the hlmt's
/// variants reference, locate the exact UE `SK_` mesh by name and load it,
/// bound to that authoritative region/permutation. The mesh naming encodes
/// the permutation (`SK_Marine_Torso_01` = region `body`, perm `torso_01`);
/// the head's `default` face/skin lives in the character's `anatomy` mesh.
/// Meshes the game keeps as `SM_` static (helmets/armor) or MetaHuman (faces)
/// aren't `SK_` and simply don't resolve here (a known gap).
fn ce_load_variant_meshes(
    containers: &[MountedContainer],
    char_root: &str,
    needed: &std::collections::BTreeSet<(String, String)>,
) -> Vec<(
    String,
    String,
    String,
    std::sync::Arc<SkeletalMesh>,
    Vec<String>,
)> {
    use std::collections::BTreeMap;
    let root_slash = format!("{char_root}/");
    let char_name = char_root.rsplit('/').next().unwrap_or("").to_string();

    // Index SK_ meshes under char_root by stem (excluding female/overlay/etc).
    let mut sk: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut anatomy: Option<String> = None;
    for (ci, c) in containers.iter().enumerate() {
        for e in c.archive.entries() {
            let norm = e.path.to_ascii_lowercase().replace('\\', "/");
            if !norm.contains(&root_slash) || !norm.ends_with(".uasset") {
                continue;
            }
            let base = norm.rsplit('/').next().unwrap_or("");
            if !base.starts_with("sk_") || ce_is_excluded(&norm, base) || base.contains("female") {
                continue;
            }
            let stem = base.strip_suffix(".uasset").unwrap_or(base).to_string();
            if stem.contains("anatomy") && anatomy.is_none() {
                anatomy = Some(stem.clone());
            }
            sk.entry(stem).or_insert((ci, e.path.clone()));
        }
    }

    // Resolve a (region, perm) to a mesh stem.
    let resolve = |region: &str, perm: &str| -> Option<String> {
        if region == "head" && perm == "default" {
            return anatomy.clone();
        }
        let exact = format!("sk_{char_name}_{perm}");
        if sk.contains_key(&exact) {
            return Some(exact);
        }
        // Multi-token perms (e.g. `torso_01`) are region-specific → match a
        // stem ending with `_<perm>`. Single-token generic perms (`default`,
        // `pilot`) only match the exact form, so `armor=default` stays empty.
        if perm.contains('_') {
            let want = format!("_{perm}");
            let mut cands: Vec<&String> = sk.keys().filter(|s| s.ends_with(&want)).collect();
            cands.sort_by_key(|s| s.len());
            return cands.first().map(|s| (*s).clone());
        }
        None
    };

    let mut out = Vec::new();
    for (region, perm) in needed {
        let Some(stem) = resolve(region, perm) else {
            continue;
        };
        let Some((ci, path)) = sk.get(&stem).cloned() else {
            continue;
        };
        let Ok(bytes) = containers[ci].archive.read(&path) else {
            continue;
        };
        let Ok(hdr) =
            FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None)
        else {
            continue;
        };
        let names = hdr.name_map.copy_raw_names();
        let Ok(mesh) = SkeletalMesh::from_package(&bytes, &names, hdr.summary.header_size as usize)
        else {
            continue;
        };
        let mats = hdr
            .imported_package_names
            .iter()
            .filter(|p| {
                let b = p.rsplit('/').next().unwrap_or("");
                b.starts_with("MI_") || b.starts_with("M_")
            })
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
            .collect();
        out.push((
            region.clone(),
            perm.clone(),
            stem,
            std::sync::Arc::new(mesh),
            mats,
        ));
    }
    out
}

/// Non-renderable / overlay meshes to skip (skeleton bind mesh, shield/shadow
/// proxies, cloth AnimDynamics, damage states, collision/physics/imposters).
fn ce_is_excluded(norm: &str, base: &str) -> bool {
    norm.contains("/skeleton/")
        || [
            "shield",
            "shadow",
            "animdynamics",
            "destroyed",
            "_dmg",
            "damage",
            "collision",
            "physics",
            "imposter",
        ]
        .iter()
        .any(|k| base.contains(k))
}

pub(super) fn expand_preview_bounds_local(min: &mut [f32; 3], max: &mut [f32; 3], point: [f32; 3]) {
    for axis in 0..3 {
        min[axis] = min[axis].min(point[axis]);
        max[axis] = max[axis].max(point[axis]);
    }
}

pub(super) struct RawVariantRegion {
    pub(super) perm: Option<String>,
    pub(super) parent: i128,
}
pub(super) struct RawVariant {
    pub(super) name: String,
    pub(super) regions: Vec<(String, RawVariantRegion)>,
}

#[cfg(test)]
mod ce_repro_tests {
    use super::*;
    use std::path::PathBuf;

    fn test_jms_vertex(uv: [f32; 2]) -> blam_tags::jms::JmsVertex {
        blam_tags::jms::JmsVertex {
            position: blam_tags::math::RealPoint3d::default(),
            normal: blam_tags::math::RealVector3d::default(),
            tangent: None,
            binormal: None,
            node_sets: Vec::new(),
            uvs: vec![blam_tags::math::RealPoint2d { x: uv[0], y: uv[1] }],
            color: None,
        }
    }

    #[test]
    fn campaign_evolved_export_copies_chimp_uv_conventions_at_the_part_offset() {
        let mut target = blam_tags::jms::JmsFile {
            vertices: vec![
                test_jms_vertex([0.0, 0.0]),
                test_jms_vertex([0.0, 0.0]),
                test_jms_vertex([0.0, 0.0]),
            ],
            ..Default::default()
        };
        let source = blam_tags::jms::JmsFile {
            vertices: vec![test_jms_vertex([2.25, -0.5]), test_jms_vertex([3.0, 1.5])],
            ..Default::default()
        };

        copy_chimp_jms_uvs(&mut target, 1, &source).unwrap();

        assert_eq!(target.vertices[0].uvs[0].x, 0.0);
        assert_eq!(target.vertices[1].uvs[0].x, 2.25);
        assert_eq!(target.vertices[1].uvs[0].y, -0.5);
        assert_eq!(target.vertices[2].uvs[0].x, 3.0);
        assert_eq!(target.vertices[2].uvs[0].y, 1.5);
    }

    /// The index narrows a package lookup to entries with its file name, then
    /// keeps the linear scan's rule and order. Checked against that scan over
    /// paths built to trip it: case, backslashes, a plugin's copy of the same
    /// file, a longer name sharing the tail, and a `.ubulk` beside the asset.
    #[test]
    fn a_package_lookup_through_the_index_finds_what_a_scan_found() {
        let paths = [
            (0, 0, "Meteorite/Content/A/SK_Foo.ubulk"),
            (0, 1, "Meteorite/Plugins/P/Content/A/SK_Foo.uasset"),
            (0, 2, "Meteorite/Content/A/SK_Foo.uasset"),
            (1, 0, "Meteorite\\Content\\B\\sk_foo.uasset"),
            (1, 1, "Meteorite/Content/B/XSK_Foo.uasset"),
            (1, 2, "Meteorite/Content/MeshSync/DA_Foo.uasset"),
        ];
        let index = index_ce_paths(paths.iter().copied());
        let normalized = |path: &str| path.to_ascii_lowercase().replace('\\', "/");
        for package in [
            "/Game/A/SK_Foo",
            "/Game/B/SK_FOO",
            "/Game/SK_Foo",
            "/Game/Nope",
            "/Game/B/Foo",
        ] {
            let (file_name, suffix) = ce_package_file_name_and_suffix(package);
            let indexed = index
                .by_file_name
                .get(&file_name)
                .into_iter()
                .flatten()
                .find(|&&(c, e)| {
                    let path = paths.iter().find(|p| (p.0, p.1) == (c, e)).unwrap().2;
                    normalized(path).ends_with(&suffix)
                })
                .copied();
            let scanned = paths
                .iter()
                .find(|p| normalized(p.2).ends_with(&suffix))
                .map(|p| (p.0, p.1));
            assert_eq!(indexed, scanned, "{package}");
        }
        assert_eq!(index.mesh_sync.len(), 1);
    }

    /// The indexed package lookups find exactly the entry the linear scans
    /// they replaced did, sampled across a real install.
    #[test]
    fn indexed_package_lookups_match_a_linear_scan() {
        let paks = crate::test_kits::ce_paks();
        if !paks.is_dir() {
            eprintln!(
                "skipping: Campaign Evolved not present at {}",
                paks.display()
            );
            return;
        }
        let loaded = crate::core::source::load_iostore_container_set(
            paks,
            &TagNameIndex::default(),
            crate::test_kits::definitions(),
        )
        .expect("mount Campaign Evolved");
        let TagSource::IoStoreContainerSet { containers, .. } = &loaded.source else {
            panic!("not a container set");
        };
        // What `ce_read_uasset_by_package` did before the index.
        let scan = |package: &str| {
            let tail = package.to_ascii_lowercase().replace('\\', "/");
            let tail = tail.strip_prefix("/game/").unwrap_or(&tail).to_owned();
            let suffix = format!("/{tail}.uasset");
            containers.iter().find_map(|c| {
                c.archive
                    .entries()
                    .iter()
                    .find(|e| {
                        e.path
                            .to_ascii_lowercase()
                            .replace('\\', "/")
                            .ends_with(&suffix)
                    })
                    .map(|e| e.path.clone())
            })
        };
        let assets: Vec<String> = containers
            .iter()
            .flat_map(|c| c.archive.entries().iter().map(|e| e.path.clone()))
            .filter(|path| path.to_ascii_lowercase().ends_with(".uasset"))
            .collect();
        let mut compared = 0;
        for path in assets.iter().step_by(500) {
            let Some((_, rest)) = path.split_once("/Content/") else {
                continue;
            };
            let package = format!("/Game/{}", rest.trim_end_matches(".uasset"));
            let found = ce_package_entries(containers, &package)
                .next()
                .map(|(_, e)| e.path.clone());
            assert_eq!(found, scan(&package), "{package}");
            compared += 1;
        }
        assert!(compared > 50, "compared only {compared} packages");
    }

    /// Runs the exact app CE-preview path against the optional `CE_PAKS`
    /// installation. `CE_MODEL` selects the tag path fragment and `CE_HD`
    /// enables Nanite detail. Skips when `CE_PAKS` is not configured.
    #[test]
    fn ce_model_real_path() {
        let Some(paks) = std::env::var_os("CE_PAKS").map(PathBuf::from) else {
            eprintln!("skip: set CE_PAKS to a Campaign Evolved Paks directory");
            return;
        };
        if !paks.exists() {
            eprintln!("skip: CE_PAKS does not exist: {}", paks.display());
            return;
        }
        let defs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let loaded =
            crate::core::source::load_iostore_container_set(paks, &TagNameIndex::default(), &defs)
                .expect("mount CE container set");
        let source = &loaded.source;
        let hlmt = u32::from_be_bytes(*b"hlmt");
        let entry = loaded
            .entries
            .iter()
            .chain(loaded.all_entries.iter())
            .find(|e| {
                e.group_tag == hlmt
                    && e.display_path.to_ascii_lowercase().contains(
                        &std::env::var("CE_MODEL").unwrap_or_else(|_| "pelican/pelican".into()),
                    )
            })
            .expect("Campaign Evolved model entry")
            .clone();
        eprintln!(
            "[TEST] entry: {} loc={:?}",
            entry.display_path,
            std::mem::discriminant(&entry.location)
        );
        let tag = read_entry(source, &entry).expect("read Campaign Evolved model");
        unsafe {
            std::env::set_var("CE_DEBUG", "1");
        }
        let data = load_campaign_evolved_preview(
            &tag,
            &entry,
            Some(source),
            std::env::var("CE_HD").is_ok(),
        )
        .expect("recognized as CE model")
        .expect("CE preview built");
        eprintln!(
            "[TEST] preview bounds min{:?} max{:?}  ({} draw tris)",
            data.preview.bounds_min,
            data.preview.bounds_max,
            data.preview.indices.len() / 3
        );
        if std::env::var("CE_JMS").is_ok() {
            let skeleton = tag_ref_path(&tag.root(), "skeleton model")
                .expect("Campaign Evolved model has a skeleton model");
            let jms = campaign_evolved_render_jms(&tag, &entry, source, &skeleton)
                .expect("Campaign Evolved Chimp JMS export");
            assert!(!jms.vertices.is_empty());
            assert!(!jms.triangles.is_empty());
            assert!(jms.vertices.iter().all(|vertex| {
                vertex
                    .uvs
                    .iter()
                    .all(|uv| uv.x.is_finite() && uv.y.is_finite())
            }));
            eprintln!(
                "[TEST] Chimp JMS export: {} vertices, {} triangles",
                jms.vertices.len(),
                jms.triangles.len()
            );
        }
        // Optional OBJ export of the exact preview geometry (set CE_OBJ).
        if std::env::var("CE_OBJ").is_ok() {
            use std::io::Write;
            let out = std::env::temp_dir().join(format!(
                "baboon-ce-preview-{}.obj",
                std::env::var("CE_MODEL")
                    .unwrap_or_else(|_| "pelican".into())
                    .replace(['/', '\\'], "_")
            ));
            let mut f = std::fs::File::create(&out).unwrap();
            for vertex in &data.preview.vertices {
                let pos = vertex.position;
                writeln!(f, "v {} {} {}", pos[0], pos[1], pos[2]).unwrap();
            }
            for triangle in data.preview.indices.chunks_exact(3) {
                writeln!(
                    f,
                    "f {} {} {}",
                    triangle[0] + 1,
                    triangle[1] + 1,
                    triangle[2] + 1
                )
                .unwrap();
            }
            eprintln!(
                "[TEST] wrote {} ({} tris)",
                out.display(),
                data.preview.indices.len() / 3
            );
        }
    }
}

#[cfg(test)]
mod particle_model_preview_tests {
    //! `particle_model` tags get a working Model Preview tab.
    //!
    //! blam-tags owns the decode (splitting the merged triangle strip at the
    //! `m_gpu_data/m_variants` boundaries, decompressing through the
    //! compression bounds) and is tested there. What this asserts is
    //! Baboon's half:
    //!
    //! - the tab pair and viewport actually appear for `pmdf` / `PRTM`,
    //! - **without** widening [`is_model_group`], whose other job is deciding
    //!   whether a `tag_reference` is an object's model link — a `particle`
    //!   tag's `Model` → `pmdf` field would be misread as one,
    //! - each JMI object becomes its own preview region, so the region list
    //!   doubles as an object toggle,
    //! - the geometry the viewport uploads is right way round — batches
    //!   index inside the vertex buffer and face normals agree with the
    //!   stored vertex normals.
    //!
    //! Skips silently when the corresponding tag set is absent.

    use std::path::PathBuf;
    use crate::core::tag_key::file_entry_key;
    use crate::core::game::GameId;

    use blam_tags::TagFile;

    use crate::app::editor::{
        is_model_group, is_previewable_geometry_group, is_previewable_geometry_group_for_game,
    };
    use crate::app::model_preview::RenderModelPreview;
    use crate::app::model_preview::loading::build_particle_model_preview;

    /// Root of an extracted MCC tag set, via `BLAM_TEST_<KIT>_TAGS` or the
    /// conventional local layout.
    fn kit_tags(kit: &str) -> Option<PathBuf> {
        let var = format!("BLAM_TEST_{}_TAGS", kit.to_uppercase());
        if let Ok(p) = std::env::var(&var) {
            let p = PathBuf::from(p);
            return p.is_dir().then_some(p);
        }
        let home = std::env::var("HOME").ok()?;
        let p = PathBuf::from(home)
            .join("Halo")
            .join(format!("{kit}_mcc"))
            .join("tags");
        p.is_dir().then_some(p)
    }

    fn names() -> crate::core::format::TagNameIndex {
        let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        crate::core::format::TagNameIndex::load_from_definitions(&defs)
    }

    /// Read a tag, routing Halo 2's classic format through its JSON
    /// definition (classic tags carry no embedded `blay`).
    fn read(path: &std::path::Path, game: &str) -> TagFile {
        let bytes = std::fs::read(path).expect("read tag bytes");
        if blam_tags::classic::ClassicHeader::parse(&bytes).is_some() {
            let def = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("definitions")
                .join(game)
                .join("particle_model.json");
            let layout = blam_tags::layout::TagLayout::from_json(&def).expect("load classic layout");
            return blam_tags::classic::read_classic_tag_file(&bytes, layout).expect("decode classic");
        }
        TagFile::read(path).expect("read tag")
    }

    /// Mean dot(face normal, averaged vertex normal) over the preview's
    /// triangles. A correct upload lands near +1; flipped winding lands near
    /// -1, and a mis-split strip near 0.
    fn face_normal_agreement(preview: &RenderModelPreview) -> Option<f32> {
        let mut total = 0.0f64;
        let mut n = 0usize;
        for tri in preview.indices.chunks_exact(3) {
            let v: Vec<_> = tri
                .iter()
                .filter_map(|&i| preview.vertices.get(i as usize))
                .collect();
            if v.len() != 3 {
                continue;
            }
            let (pa, pb, pc) = (v[0].position, v[1].position, v[2].position);
            let u = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let w = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            let f = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            let fl = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
            if fl < 1e-12 {
                continue;
            }
            let vn = [
                (v[0].normal[0] + v[1].normal[0] + v[2].normal[0]) / 3.0,
                (v[0].normal[1] + v[1].normal[1] + v[2].normal[1]) / 3.0,
                (v[0].normal[2] + v[1].normal[2] + v[2].normal[2]) / 3.0,
            ];
            let vl = (vn[0] * vn[0] + vn[1] * vn[1] + vn[2] * vn[2]).sqrt();
            if vl < 1e-12 {
                continue;
            }
            total += (0..3).map(|k| (f[k] / fl) * (vn[k] / vl)).sum::<f32>() as f64;
            n += 1;
        }
        (n > 0).then(|| (total / n as f64) as f32)
    }

    /// The panel gate opens for particle models — and `is_model_group` stays
    /// closed, so `find_model_reference` does not start treating a
    /// `particle`'s `Model` field as an object's model link.
    #[test]
    fn particle_model_is_previewable_without_becoming_a_model() {
        let names = names();
        for group in [b"pmdf", b"PRTM"] {
            let tag = u32::from_be_bytes(*group);
            let label = String::from_utf8_lossy(group).into_owned();
            assert!(
                is_previewable_geometry_group(tag, &names),
                "`{label}` must open the Model Preview tab",
            );
            assert!(
                !is_model_group(tag, &names),
                "`{label}` must NOT count as a model group — that predicate also \
             decides whether a tag_reference is an object's model link",
            );
        }
        // The predicate must still admit everything it used to.
        for group in [b"hlmt", b"mod2"] {
            let tag = u32::from_be_bytes(*group);
            assert!(is_previewable_geometry_group(tag, &names));
        }
    }

    /// The panel gate opens for a bare `render_model` (mode) — the preview loader
    /// draws the tag itself, so the viewport belongs inside the tag too.
    #[test]
    fn render_model_is_previewable() {
        let names = names();
        let tag = u32::from_be_bytes(*b"mode");
        assert!(
            is_previewable_geometry_group(tag, &names),
            "`mode` must open the Model Preview tab",
        );
    }

    #[test]
    fn object_family_preview_is_halo_ce_only() {
        let names = names();
        for group in [b"bipd", b"vehi", b"weap", b"eqip", b"scen"] {
            let tag = u32::from_be_bytes(*group);
            assert!(is_previewable_geometry_group_for_game(
                tag,
                &names,
                Some(GameId::HaloCe)
            ));
            assert!(!is_previewable_geometry_group_for_game(
                tag,
                &names,
                Some(GameId::Halo3)
            ));
            assert!(!is_previewable_geometry_group_for_game(tag, &names, None));
        }
    }

    /// A multi-object gen3 tag: one region per JMI object, batches wired to
    /// those regions, and geometry the right way round.
    #[test]
    fn gen3_objects_become_regions_with_valid_geometry() {
        let Some(tags) = kit_tags("haloreach") else {
            return;
        };
        let path = tags.join("fx/particles/models/debris/generic_shards/generic_shards.particle_model");
        if !path.is_file() {
            return;
        }
        let tag = read(&path, "haloreach_mcc");
        let preview = build_particle_model_preview(&tag, "generic_shards").expect("build preview");

        assert_eq!(preview.regions.len(), 8, "generic_shards ships 8 objects");
        assert_eq!(
            preview.batches.len(),
            preview.regions.len(),
            "every object needs a draw batch or it renders invisible",
        );
        for (region, batch) in preview.regions.iter().zip(&preview.batches) {
            assert_eq!(
                region.name, batch.region_name,
                "batch must target its region"
            );
            assert!(
                region.permutations.contains(&batch.permutation_name),
                "batch permutation `{}` is not selectable in region `{}`",
                batch.permutation_name,
                region.name,
            );
            assert!(
                batch.index_count > 0,
                "region `{}` has an empty batch",
                region.name
            );
        }

        // Every batch must address inside the shared buffers, or the
        // renderer reads past the end.
        for batch in &preview.batches {
            let end = (batch.index_start + batch.index_count) as usize;
            assert!(
                end <= preview.indices.len(),
                "batch range past the index buffer"
            );
            for &i in &preview.indices[batch.index_start as usize..end] {
                assert!(
                    (i as usize) < preview.vertices.len(),
                    "index past the vertex buffer"
                );
            }
        }

        assert!(
            preview.bounds_min.iter().all(|v| v.is_finite())
                && preview.bounds_max.iter().all(|v| v.is_finite()),
            "bounds must be finite or the camera cannot frame the model",
        );

        let score = face_normal_agreement(&preview).expect("measurable");
        assert!(
            score > 0.6,
            "preview geometry scored {score:.3} — a mis-split strip scores ~0 and \
         flipped winding ~-1",
        );
    }

    /// Halo 2's `PRTM` is a different tag and a different decode, and it
    /// names its own objects — the region list should show those names.
    #[test]
    fn halo2_regions_carry_the_shipped_object_names() {
        let Some(tags) = kit_tags("halo2") else {
            return;
        };
        let path = tags.join("effects/particle_models/urban_debris/urban_debris.particle_model");
        if !path.is_file() {
            return;
        }
        let tag = read(&path, "halo2_mcc");
        let preview = build_particle_model_preview(&tag, "urban_debris").expect("build preview");

        let region_names: Vec<&str> = preview.regions.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            region_names,
            vec![
                "can_1", "can_2", "can_3", "can_4", "can_5", "paper_1", "paper_2", "paper_3", "butt_1",
                "butt_2",
            ],
            "Halo 2 stores `models[].model name` — the region list must show them",
        );

        let score = face_normal_agreement(&preview).expect("measurable");
        assert!(score > 0.6, "preview geometry scored {score:.3}");
    }

    /// A single-object tag still produces one selectable region rather than
    /// an empty list, so the viewport is not blank.
    #[test]
    fn single_object_tag_still_yields_one_region() {
        let Some(tags) = kit_tags("haloreach") else {
            return;
        };
        let path = tags.join("fx/particles/models/weapons/brute_spike/brute_spike.particle_model");
        if !path.is_file() {
            return;
        }
        let tag = read(&path, "haloreach_mcc");
        let preview = build_particle_model_preview(&tag, "brute_spike").expect("build preview");

        assert_eq!(preview.regions.len(), 1);
        assert_eq!(preview.regions[0].name, "brute_spike");
        assert!(
            !preview.indices.is_empty(),
            "single-object preview must have geometry"
        );
    }

    /// Drive the real UI entry point, not the builder underneath it.
    ///
    /// `load_model_preview` is what the panel calls, and it derives the
    /// object-naming stem from `entry.display_path` rather than being handed
    /// one. Asserting through it is what catches a stem derivation that
    /// silently yields `""` (every object would become `_1`, `_2`, …) or
    /// keeps the `.particle_model` extension.
    #[test]
    fn load_model_preview_derives_object_names_from_the_entry() {
        let Some(tags) = kit_tags("haloreach") else {
            return;
        };
        let path = tags.join("fx/particles/models/debris/falling_leaves/falling_leaves.particle_model");
        if !path.is_file() {
            return;
        }
        let tag = read(&path, "haloreach_mcc");
        let names = names();
        let entry = crate::core::source::TagEntry {
            key: file_entry_key(&path),
            display_path: "fx/particles/models/debris/falling_leaves/falling_leaves.particle_model"
                .to_owned(),
            group_tag: tag.header.group_tag,
            group_name: Some("particle_model".to_owned()),
            location: crate::core::source::TagEntryLocation::LooseFile(path.clone()),
        };

        let data = crate::app::model_preview::loading::load_model_preview(
            &tag,
            &entry,
            &names,
            None,
            &crate::app::model_preview::loading::PreviewLoadSettings::default(),
        )
        .expect("preview loads without a loose-folder source — geometry is inline");

        assert!(!data.preview.regions.is_empty(), "no objects were exposed");
        for region in &data.preview.regions {
            assert!(
                region.name.starts_with("falling_leaves"),
                "object `{}` was not named from the tag stem — the stem derivation \
             is dropping or mangling `entry.display_path`",
                region.name,
            );
            assert!(
                !region.name.contains(".particle_model"),
                "object `{}` kept the tag extension",
                region.name,
            );
        }
        // A particle_model has no model variants. The Variant combo still
        // renders (showing only `<None>`), same as a bare `render_model`
        // preview — but nothing must invent entries for it, or the combo
        // would offer selections that change nothing.
        assert!(data.variants.is_empty(), "a particle_model has no variants");
    }

    /// A shipped Halo 3 `render_model` opened on its own must produce a preview
    /// with geometry: `load_model_preview` draws the tag itself, no `hlmt`
    /// wrapper involved, which is what the Model Preview tab inside the tag shows.
    #[test]
    fn a_shipped_render_model_previews_on_its_own() {
        let Some(tags) = kit_tags("halo3") else {
            eprintln!("skipping: no halo3 tag set");
            return;
        };
        let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
        let path = tags.join(rel);
        if !path.is_file() {
            eprintln!("skipping: no {rel} in this kit");
            return;
        }
        let tag = read(&path, "halo3_mcc");
        let names = names();
        assert!(
            is_previewable_geometry_group(tag.header.group_tag, &names),
            "`mode` must open the Model Preview tab",
        );
        let entry = crate::core::source::TagEntry {
            key: file_entry_key(&path),
            display_path: rel.to_owned(),
            group_tag: tag.header.group_tag,
            group_name: Some("render_model".to_owned()),
            location: crate::core::source::TagEntryLocation::LooseFile(path.clone()),
        };
        let data = crate::app::model_preview::loading::load_model_preview(
            &tag,
            &entry,
            &names,
            None,
            &crate::app::model_preview::loading::PreviewLoadSettings::default(),
        )
        .expect("a shipped render_model must preview");
        assert!(
            !data.preview.batches.is_empty(),
            "the preview came back with no draw batches"
        );
    }

    /// Every shipped particle_model in every present kit must produce a
    /// preview with geometry — no panic, no empty viewport.
    ///
    /// The panel wraps the load in `catch_unwind` and shows the message, so
    /// a regression here degrades to a red label rather than a crash; this
    /// keeps it from degrading silently.
    #[test]
    fn every_shipped_particle_model_previews() {
        let kits = [
            ("halo2", "halo2_mcc"),
            ("halo3", "halo3_mcc"),
            ("haloreach", "haloreach_mcc"),
            ("halo4", "halo4_mcc"),
        ];
        let names = names();
        let mut checked = 0usize;
        let mut objects = 0usize;
        let mut failures: Vec<String> = Vec::new();

        for (kit, game) in kits {
            let Some(root) = kit_tags(kit) else { continue };
            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    if path.extension().and_then(|e| e.to_str()) != Some("particle_model") {
                        continue;
                    }
                    let tag = read(&path, game);
                    let rel = path.strip_prefix(&root).unwrap_or(&path);
                    let display = rel.to_string_lossy().replace('\\', "/");
                    let tag_entry = crate::core::source::TagEntry {
                        key: file_entry_key(&path),
                        display_path: display.clone(),
                        group_tag: tag.header.group_tag,
                        group_name: Some("particle_model".to_owned()),
                        location: crate::core::source::TagEntryLocation::LooseFile(path.clone()),
                    };
                    checked += 1;
                    match crate::app::model_preview::loading::load_model_preview(
                        &tag,
                        &tag_entry,
                        &names,
                        None,
                        &crate::app::model_preview::loading::PreviewLoadSettings::default(),
                    ) {
                        Ok(data) => {
                            if data.preview.indices.is_empty() || data.preview.regions.is_empty() {
                                failures.push(format!("{display}: empty preview"));
                            }
                            objects += data.preview.regions.len();
                        }
                        Err(e) => failures.push(format!("{display}: {e}")),
                    }
                }
            }
        }

        if checked == 0 {
            return; // no kits present
        }
        assert!(
            failures.is_empty(),
            "{} of {checked} particle_models failed to preview:\n  {}",
            failures.len(),
            failures.join("\n  "),
        );
        eprintln!("[particle_model preview] {checked} tags, {objects} objects");
    }
}

#[cfg(test)]
mod model_preview_worker_tests {
    //! The model preview parses on a worker, not the UI thread.
    //!
    //! What the app derives, frame by frame: the post-draw hook starts a worker
    //! and leaves the state without data (the loading shells), the worker's
    //! message installs the result, and a result for a request the state no
    //! longer wants is dropped instead of installed. Both a modern (Halo 3) and a
    //! classic (Halo 2) tag must load, from disk and from an edited document's
    //! bytes: a classic tag's bytes carry no layout, so re-parsing them the
    //! modern way fails where reading them the kit's way does not.
    //!
    //! Needs `BLAM_TEST_H3EK` / `BLAM_TEST_H2EK`; skips a kit that is not set.

    use std::path::Path;
    use crate::core::tag_key::file_entry_key;
    use crate::core::game::GameId;
    use std::time::{Duration, Instant};

    use eframe::egui;

    use crate::app::model_preview::state::ModelTagPanelTab;
    use crate::app::{Baboon, LoadedSourceData, ModelPreviewState, TagDocument};
    use crate::core::format::TagNameIndex;
    use crate::core::source::{TagEntry, TagEntryLocation, TagSource, TagTree};

    struct Fixture {
        app: Baboon,
        key: String,
        ctx: egui::Context,
    }

    fn fixture(tags: &Path, game: &str, rel: &str) -> Option<Fixture> {
        let path = tags.join(rel);
        if !path.is_file() {
            eprintln!("skipping: {} is not present", path.display());
            return None;
        }
        let definitions = crate::test_kits::definitions();
        let entry = TagEntry {
            key: file_entry_key(&path),
            display_path: rel.to_owned(),
            group_tag: u32::from_be_bytes(*b"mode"),
            group_name: Some("render_model".to_owned()),
            location: TagEntryLocation::LooseFile(path.clone()),
        };
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: game.to_owned(),
            source: TagSource::LooseFolder {
                root: tags.to_path_buf(),
                game: GameId::from_id(game),
                definitions_root: definitions.to_path_buf(),
            },
            names: TagNameIndex::load_from_definitions(definitions),
            game: GameId::from_id(game),
            entries: vec![entry.clone()],
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: vec![entry.clone()],
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: true,
            chosen_kit_layout: None,
        });
        let preview = ModelPreviewState {
            active_tab: ModelTagPanelTab::ModelPreview,
            ..ModelPreviewState::default()
        };
        app.views[app.model.kits[0].id].caches.model_previews.insert(entry.key.clone(), preview);
        Some(Fixture {
            app,
            key: entry.key,
            ctx: egui::Context::default(),
        })
    }

    impl Fixture {
        fn state(&self) -> &ModelPreviewState {
            &self.app.views[self.app.model.kits[0].id].caches.model_previews[&self.key]
        }

        fn state_mut(&mut self) -> &mut ModelPreviewState {
            self.app.views[self.app.model.kits[0].id].caches.model_previews.get_mut(&self.key).unwrap()
        }

        /// One frame's worth of preview work: drain replies, then the post-draw hook.
        fn frame(&mut self) {
            self.app.process_worker_messages(&self.ctx);
            let key = self.key.clone();
            self.app.maybe_request_model_preview(0, &key, &self.ctx);
        }

        /// Run frames until the state holds a preview it considers current.
        fn frames_until_loaded(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(60);
            while self.state().needs_preview_load(&self.key) {
                assert!(Instant::now() < deadline, "the preview never landed");
                self.frame();
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        /// Open the tag as an edited document, so the worker parses its bytes.
        fn open_edited(&mut self) {
            let entry = self.app.model.kits[0].entry_for_key(&self.key).unwrap().clone();
            let source = self.app.model.kits[0].source.as_ref().unwrap().source.clone();
            let tag = crate::core::source::read_entry(&source, &entry).expect("read render_model");
            let mut document = TagDocument::clean(tag);
            document.dirty.touch();
            self.app.model.kits[0]
                .parsed_tags
                .insert(self.key.clone(), document);
        }
    }

    fn loads_on_a_worker(fixture: &mut Fixture) {
        fixture.frame();
        assert!(
            fixture.state().data.is_none() && fixture.state().preview_load_id.is_some(),
            "the first frame must hand the parse to a worker and show the shells"
        );
        fixture.frames_until_loaded();

        let data = fixture
            .state()
            .data
            .as_ref()
            .unwrap()
            .as_ref()
            .expect("preview loads");
        assert!(!data.preview.batches.is_empty(), "no draw batches");
    }

    fn check(tags: &Path, game: &str, rel: &str) {
        for edited in [false, true] {
            if let Some(mut fixture) = fixture(tags, game, rel) {
                if edited {
                    fixture.open_edited();
                }
                loads_on_a_worker(&mut fixture);
            }
        }
    }

    #[test]
    fn a_halo3_render_model_loads_on_a_worker() {
        check(
            &crate::test_kits::h3ek_tags(),
            "halo3_mcc",
            "objects/weapons/rifle/assault_rifle/assault_rifle.render_model",
        );
    }

    #[test]
    fn a_classic_halo2_render_model_loads_on_a_worker() {
        check(
            &crate::test_kits::h2ek_tags(),
            "halo2_mcc",
            "objects/weapons/rifle/battle_rifle/battle_rifle.render_model",
        );
    }

    /// Wait for the worker's reply without handing it to the app.
    fn wait_for_reply(fixture: &Fixture) -> crate::app::WorkerMessage {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Ok(message) = fixture.app.rx.try_recv() {
                return message;
            }
            assert!(Instant::now() < deadline, "the worker never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A setting changed while the worker ran: its result answers a request the
    /// state no longer makes, so it must be dropped and the load re-run — not
    /// installed as current and left for a later frame to notice.
    #[test]
    fn a_result_for_a_superseded_request_is_dropped() {
        let tags = crate::test_kits::h3ek_tags();
        let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
        let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
            return;
        };
        assert!(fixture.state().high_detail);
        fixture.frame();
        let first = fixture.state().preview_load_id.expect("a worker started");
        // Let the superseded worker finish, so dropping its result is a choice
        // the app makes rather than a race it happened to win.
        let reply = wait_for_reply(&fixture);

        fixture.state_mut().high_detail = false;
        let key = fixture.key.clone();
        fixture.app.maybe_request_model_preview(0, &key, &fixture.ctx);
        let second = fixture.state().preview_load_id.expect("a new worker started");
        assert_ne!(first, second);
        fixture.app.tx.send(reply).unwrap();
        fixture.app.process_worker_messages(&fixture.ctx);
        assert!(
            fixture.state().data.is_none(),
            "the superseded result was installed as the preview"
        );

        fixture.frames_until_loaded();
        assert!(!fixture.state().loaded_high_detail);
    }

    /// An edit invalidates the preview while a load is in flight: the worker is
    /// parsing the bytes from before the edit, so its result must not land.
    #[test]
    fn invalidating_drops_the_load_in_flight() {
        let tags = crate::test_kits::h3ek_tags();
        let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
        let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
            return;
        };
        fixture.frame();
        let reply = wait_for_reply(&fixture);
        fixture.state_mut().invalidate_load();
        fixture.app.tx.send(reply).unwrap();
        fixture.app.process_worker_messages(&fixture.ctx);
        assert!(
            fixture.state().data.is_none(),
            "the pre-edit load landed after the edit"
        );
    }

    /// The kit's generation moved while the worker ran — a background scan
    /// started, or a tag was created. The reply is stale and is dropped, but the
    /// preview must ask again rather than wait on a request nobody will answer.
    #[test]
    fn a_result_dropped_for_a_generation_bump_is_requested_again() {
        let tags = crate::test_kits::h3ek_tags();
        let rel = "objects/weapons/rifle/assault_rifle/assault_rifle.render_model";
        let Some(mut fixture) = fixture(&tags, "halo3_mcc", rel) else {
            return;
        };
        fixture.frame();
        let first = fixture.state().preview_load_id.expect("a worker started");
        let reply = wait_for_reply(&fixture);
        fixture.app.model.kits[0].generation = fixture.app.model.kits[0].generation.wrapping_add(1);
        fixture.app.tx.send(reply).unwrap();
        fixture.app.process_worker_messages(&fixture.ctx);
        assert!(fixture.state().data.is_none(), "a stale result was installed");

        let key = fixture.key.clone();
        fixture.app.maybe_request_model_preview(0, &key, &fixture.ctx);
        let second = fixture.state().preview_load_id;
        assert!(
            second.is_some_and(|second| second != first),
            "the dropped request was never re-made: the preview waits for good"
        );
        fixture.frames_until_loaded();
    }
}

#[cfg(test)]
mod synthetic_loading_tests {
    //! `load_model_preview` over synthetic tags: which group goes down which
    //! path, what each produces, and what each refuses with.
    //!
    //! Characterization, with no kit. The geometry is a Halo CE gbxmodel built
    //! from the definitions — the one render format whose vertices and triangles
    //! are plain tag blocks — and the references between tags resolve against a
    //! loose folder written to a temporary directory.

    use super::*;
    use crate::app::{Baboon, LoadedSourceData, ModelPreviewState, TagDocument};
    use crate::app::model_preview::state::ModelTagPanelTab;
    use crate::core::source::TagTree;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn new_tag_for(game: &str, group: &str) -> TagFile {
        TagFile::new(
            crate::core::bundled::locate_definitions_root()
                .join(game)
                .join(format!("{group}.json")),
        )
        .unwrap_or_else(|error| panic!("{game}/{group}.json: {error:?}"))
    }

    /// A fresh Halo CE tag of `group` in its classic container.
    ///
    /// `TagFile::new` only builds MCC containers, and a CE tag in one is not
    /// read as Halo CE by anything downstream. So this assembles the smallest
    /// classic file there is — the 64-byte header and an all-zero root struct —
    /// and reads it back the way a loose CE kit reads its tags.
    fn classic_ce_tag(group: &str) -> TagFile {
        let definitions = crate::core::bundled::locate_definitions_root();
        let path = definitions.join("haloce_mcc").join(format!("{group}.json"));
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read the definition")).unwrap();
        let root_block = json["block"].as_str().unwrap();
        let root_struct = json["blocks"][root_block]["struct"].as_str().unwrap();
        let size = json["structs"][root_struct]["size"].as_u64().unwrap() as usize;
        let group_tag: [u8; 4] = json["tag"].as_str().unwrap().as_bytes().try_into().unwrap();
        let version = json["version"].as_u64().unwrap() as u16;
        let mut bytes = vec![0u8; 64];
        bytes[36..40].copy_from_slice(&group_tag);
        bytes[40..44].copy_from_slice(&u32::MAX.to_be_bytes());
        bytes[56..58].copy_from_slice(&version.to_be_bytes());
        bytes[60..64].copy_from_slice(b"blam");
        bytes.resize(64 + size, 0);
        let tag = crate::core::source::read_tag_from_bytes(
            &bytes,
            Some(GameId::HaloCe),
            Some(&definitions),
            u32::from_be_bytes(group_tag),
        )
        .unwrap_or_else(|error| panic!("a fresh classic {group}: {error:#}"));
        assert_eq!(
            blam_tags::game::Game::of(&tag),
            blam_tags::game::Game::Halo1
        );
        tag
    }

    fn set(tag: &mut TagFile, path: &str, input: &str) {
        crate::core::document::apply::apply_field_edit(tag, path, input)
            .unwrap_or_else(|error| panic!("{path} = {input}: {error}"));
    }

    fn reference(tag: &mut TagFile, path: &str, group: &[u8; 4], name: &str) {
        let mut root = tag.root_mut();
        root.field_path_mut(path)
            .unwrap_or_else(|| panic!("{path} resolves"))
            .set(blam_tags::TagFieldData::TagReference(
                blam_tags::TagReferenceData {
                    group_tag_and_name: Some((u32::from_be_bytes(*group), name.to_owned())),
                },
            ))
            .unwrap_or_else(|error| panic!("{path}: {error:?}"));
    }

    fn add(tag: &mut TagFile, path: &str) {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut(path)
            .unwrap_or_else(|| panic!("{path} resolves"));
        field
            .as_block_mut()
            .unwrap_or_else(|| panic!("{path} is a block"))
            .add_element();
    }

    /// A Halo CE gbxmodel: one region `body` whose permutation `base` uses
    /// geometry 0, one part of one triangle spanning (0,0,0)-(1,2,3).
    fn gbxmodel() -> TagFile {
        let mut tag = classic_ce_tag("gbxmodel");
        add(&mut tag, "regions");
        set(&mut tag, "regions[0]/name", "body");
        add(&mut tag, "regions[0]/permutations");
        set(&mut tag, "regions[0]/permutations[0]/name", "base");
        set(&mut tag, "regions[0]/permutations[0]/super high", "0");
        add(&mut tag, "geometries");
        add(&mut tag, "geometries[0]/parts");
        for (index, position) in ["0, 0, 0", "1, 0, 0", "0, 2, 3"].into_iter().enumerate() {
            add(&mut tag, "geometries[0]/parts[0]/uncompressed vertices");
            set(
                &mut tag,
                &format!("geometries[0]/parts[0]/uncompressed vertices[{index}]/position"),
                position,
            );
            set(
                &mut tag,
                &format!("geometries[0]/parts[0]/uncompressed vertices[{index}]/normal"),
                "0, 0, 1",
            );
        }
        add(&mut tag, "geometries[0]/parts[0]/triangles");
        for (field, index) in [("vertex0 index", "0"), ("vertex1 index", "1"), ("vertex2 index", "2")]
        {
            set(
                &mut tag,
                &format!("geometries[0]/parts[0]/triangles[0]/{field}"),
                index,
            );
        }
        tag
    }

    fn entry(display_path: &str, tag: &TagFile, location: TagEntryLocation) -> TagEntry {
        TagEntry {
            key: format!("file:{display_path}"),
            display_path: display_path.to_owned(),
            group_tag: tag.header.group_tag,
            group_name: display_path
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_owned()),
            location,
        }
    }

    fn names() -> TagNameIndex {
        TagNameIndex::load_from_definitions(&crate::core::bundled::locate_definitions_root())
    }

    fn load(tag: &TagFile, display_path: &str, source: Option<&TagSource>) -> Result<ModelPreviewData, String> {
        load_model_preview(
            tag,
            &entry(display_path, tag, TagEntryLocation::LooseFile(display_path.into())),
            &names(),
            source,
            &PreviewLoadSettings::default(),
        )
    }

    /// A fresh temporary tags folder, removed when dropped.
    struct LooseKit {
        root: PathBuf,
        game: &'static str,
    }

    impl LooseKit {
        fn new(name: &str, game: &'static str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "baboon-preview-{name}-{}-{}",
                std::process::id(),
                NEXT_KIT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self { root, game }
        }

        fn write(&self, relative: &str, tag: &TagFile) -> PathBuf {
            let path = self.root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, tag.write_to_bytes().expect("serialize the tag")).unwrap();
            path
        }

        fn source(&self) -> TagSource {
            TagSource::LooseFolder {
                root: self.root.clone(),
                game: GameId::from_id(self.game),
                definitions_root: crate::core::bundled::locate_definitions_root(),
            }
        }
    }

    impl Drop for LooseKit {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    static NEXT_KIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn assert_one_triangle(preview: &RenderModelPreview) {
        assert_eq!(preview.regions.len(), 1);
        assert_eq!(preview.regions[0].name, "body");
        assert_eq!(preview.regions[0].permutations, ["base"]);
        assert_eq!(preview.vertices.len(), 3);
        assert_eq!(preview.indices.len(), 3);
        assert_eq!(preview.batches.len(), 1);
        assert_eq!(preview.bounds_min, [0.0, 0.0, 0.0]);
        assert_eq!(preview.bounds_max, [1.0, 2.0, 3.0]);
    }

    /// A gbxmodel is its own render geometry: no wrapper, no source needed.
    #[test]
    fn a_gbxmodel_previews_itself() {
        let tag = gbxmodel();
        let data = load(&tag, "objects/thing/thing.gbxmodel", None).expect("the gbxmodel previews");
        assert_eq!(data.source_key, "file:objects/thing/thing.gbxmodel");
        assert_eq!(data.render_model_path, "objects/thing/thing.gbxmodel");
        assert!(data.variants.is_empty());
        assert!(data.scenario_bsps.is_empty());
        assert_one_triangle(&data.preview);
    }

    /// Geometry with nothing to draw is refused rather than shown empty.
    #[test]
    fn an_empty_render_tag_is_refused() {
        let tag = classic_ce_tag("gbxmodel");
        assert_eq!(
            load(&tag, "objects/thing/empty.gbxmodel", None).err().as_deref(),
            Some("This render tag has no previewable draw batches.")
        );
    }

    /// A Halo CE object names its gbxmodel directly, and previews it from the
    /// loaded source — refusing without one, or without a reference.
    #[test]
    fn a_halo_ce_object_previews_the_gbxmodel_it_names() {
        let kit = LooseKit::new("ce-object", "haloce_mcc");
        kit.write("objects/thing/thing.gbxmodel", &gbxmodel());
        let source = kit.source();

        let mut scenery = classic_ce_tag("scenery");
        assert_eq!(
            load(&scenery, "objects/thing/thing.scenery", Some(&source))
                .err()
                .as_deref(),
            Some("This object references no gbxmodel.")
        );
        reference(&mut scenery, "object/model", b"mod2", "objects\\thing\\thing");
        assert_eq!(
            load(&scenery, "objects/thing/thing.scenery", None).err().as_deref(),
            Some("Halo CE object preview requires a loaded source.")
        );
        let data = load(&scenery, "objects/thing/thing.scenery", Some(&source))
            .expect("the referenced gbxmodel previews");
        assert_eq!(data.source_key, "file:objects/thing/thing.scenery");
        assert_eq!(data.render_model_path, "objects\\thing\\thing");
        assert_one_triangle(&data.preview);

        reference(&mut scenery, "object/model", b"mod2", "objects\\thing\\missing");
        let error = load(&scenery, "objects/thing/thing.scenery", Some(&source)).err().expect("refused");
        assert!(
            error.starts_with("Could not load objects\\thing\\missing.gbxmodel:"),
            "{error}"
        );
    }

    /// A `.model` resolves its render model against the loose folder, and says
    /// which of the steps on the way failed.
    #[test]
    fn a_model_resolves_its_render_model_in_the_loose_folder() {
        let kit = LooseKit::new("h3-model", "halo3_mcc");
        let source = kit.source();
        let mut model = new_tag_for("halo3_mcc", "model");
        assert_eq!(
            load(&model, "objects/thing/thing.model", Some(&source))
                .err()
                .as_deref(),
            Some("This model tag has no render model reference.")
        );
        set(&mut model, "render model", "objects\\thing\\thing.render_model");
        assert_eq!(
            load(&model, "objects/thing/thing.model", None).err().as_deref(),
            Some("Render model preview requires a loaded loose-folder editing kit.")
        );
        let error = load(&model, "objects/thing/thing.model", Some(&source)).err().expect("refused");
        assert!(
            error.starts_with("Referenced render_model was not found:"),
            "{error}"
        );
        assert!(error.ends_with("thing.render_model"), "{error}");

        // Present, but a fresh render_model has nothing to draw.
        kit.write(
            "objects/thing/thing.render_model",
            &new_tag_for("halo3_mcc", "render_model"),
        );
        assert_eq!(
            load(&model, "objects/thing/thing.model", Some(&source))
                .err()
                .as_deref(),
            Some("Referenced render_model has no previewable draw batches.")
        );
    }

    /// A scenario lists its BSPs and loads none of them until one is chosen.
    #[test]
    fn a_scenario_lists_its_bsps_without_loading_them() {
        let mut scenario = new_tag_for("halo3_mcc", "scenario");
        assert_eq!(
            load(&scenario, "levels/test/test.scenario", None)
                .err()
                .as_deref(),
            Some("This scenario lists no structure BSPs.")
        );
        add(&mut scenario, "structure bsps");
        add(&mut scenario, "structure bsps");
        set(
            &mut scenario,
            "structure bsps[0]/structure bsp",
            "levels\\test\\test_a.scenario_structure_bsp",
        );
        let data = load(&scenario, "levels/test/test.scenario", None).expect("the scenario lists");
        assert_eq!(
            data.scenario_bsps,
            [Some("levels\\test\\test_a".to_owned()), None]
        );
        assert!(data.preview.batches.is_empty(), "nothing loads unasked");
        assert_eq!(data.preview.bounds_min, [0.0; 3]);

        // Choosing one needs a source to load it from.
        let settings = PreviewLoadSettings {
            high_detail: true,
            scenario_selection: [0].into_iter().collect(),
        };
        let error = load_model_preview(
            &scenario,
            &entry(
                "levels/test/test.scenario",
                &scenario,
                TagEntryLocation::LooseFile("levels/test/test.scenario".into()),
            ),
            &names(),
            None,
            &settings,
        )
        .err().expect("refused");
        assert_eq!(error, "Scenario preview requires a loaded source.");
    }

    /// A model tag of no previewable kind falls through to the render-model
    /// lookup, and a physics model with no shapes is refused by its builder.
    #[test]
    fn other_groups_take_the_paths_their_group_names() {
        let biped = new_tag_for("halo3_mcc", "biped");
        assert_eq!(
            load(&biped, "objects/thing/thing.biped", None).err().as_deref(),
            Some("This model tag has no render model reference.")
        );
        let physics = new_tag_for("halo3_mcc", "physics_model");
        assert!(
            load(&physics, "objects/thing/thing.physics_model", None).is_err(),
            "an empty physics model has nothing to show"
        );
    }

    /// The worker round trip: the post-draw hook hands the parse to a thread,
    /// shows the loading shells, and the reply installs the preview with its
    /// selection reset — from disk, and from an edited document's bytes.
    #[test]
    fn a_gbxmodel_preview_loads_on_a_worker() {
        for edited in [false, true] {
            let kit = LooseKit::new("worker", "haloce_mcc");
            let relative = "objects/thing/thing.gbxmodel";
            let path = kit.write(relative, &gbxmodel());
            let tag = gbxmodel();
            let entry = entry(relative, &tag, TagEntryLocation::LooseFile(path));
            let mut app = Baboon::for_test();
            app.install_loaded_source(LoadedSourceData {
                label: "synthetic".to_owned(),
                source: kit.source(),
                names: names(),
                game: Some(GameId::HaloCe),
                entries: vec![entry.clone()],
                tree: TagTree::default(),
                group_tree: TagTree::default(),
                all_entries: vec![entry.clone()],
                reverse_dependencies: None,
                initial_tag: None,
                key_hints: Default::default(),
                complete_scan: true,
                chosen_kit_layout: None,
            });
            if edited {
                // Moved the triangle's apex: the worker must parse these bytes,
                // not the file.
                let mut document = TagDocument::clean(gbxmodel());
                set(
                    &mut document.tag,
                    "geometries[0]/parts[0]/uncompressed vertices[2]/position",
                    "0, 4, 5",
                );
                document.dirty.touch();
                app.model.kits[0].parsed_tags.insert(entry.key.clone(), document);
            }
            let ctx = egui::Context::default();
            // Not the preview tab: nothing is asked for.
            app.views[app.model.kits[0].id]
                .caches.model_previews
                .insert(entry.key.clone(), ModelPreviewState::default());
            app.maybe_request_model_preview(0, &entry.key, &ctx);
            assert!(app.views[app.model.kits[0].id].caches.model_previews[&entry.key].preview_load_id.is_none());

            app.views[app.model.kits[0].id]
                .caches.model_previews
                .get_mut(&entry.key)
                .unwrap()
                .active_tab = ModelTagPanelTab::ModelPreview;
            app.maybe_request_model_preview(0, &entry.key, &ctx);
            let state = &app.views[app.model.kits[0].id].caches.model_previews[&entry.key];
            let first = state.preview_load_id.expect("a worker started");
            assert!(state.data.is_none(), "the shells show while it parses");
            assert_eq!(state.loaded_key.as_deref(), Some(entry.key.as_str()));
            // Asking again while it runs starts nothing new.
            app.maybe_request_model_preview(0, &entry.key, &ctx);
            assert_eq!(
                app.views[app.model.kits[0].id].caches.model_previews[&entry.key].preview_load_id,
                Some(first)
            );

            let deadline = Instant::now() + Duration::from_secs(60);
            while app.views[app.model.kits[0].id].caches.model_previews[&entry.key].data.is_none() {
                assert!(Instant::now() < deadline, "the preview never landed");
                app.process_worker_messages(&ctx);
                std::thread::sleep(Duration::from_millis(5));
            }
            let state = &app.views[app.model.kits[0].id].caches.model_previews[&entry.key];
            assert!(state.preview_load_id.is_none(), "the request is answered");
            assert_eq!(state.render_model_path.as_deref(), Some(relative));
            let data = state.data.as_ref().unwrap().as_ref().expect("it loads");
            assert_eq!(data.preview.vertices.len(), 3);
            let apex = if edited { [1.0, 4.0, 5.0] } else { [1.0, 2.0, 3.0] };
            assert_eq!(data.preview.bounds_max, apex, "edited: {edited}");
        }
    }
}
