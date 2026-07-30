//! Preview tag loading and render-model resolution.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;
use crate::source::MountedContainer;
use blam_tags::iostore::IoStoreArchive;
use blam_tags::iostore::container_header::EIoContainerHeaderVersion;
use blam_tags::iostore::skeletal_mesh::SkeletalMesh;
use blam_tags::iostore::static_mesh::StaticMesh;
use blam_tags::iostore::ue_types::{EIoStoreTocVersion, FPackageObjectIndex};
use blam_tags::iostore::unversioned::{MeshRef, MeshSyncRegions, Permutation, PropValue, Region};
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::zen::FZenPackageHeader;
use blam_tags::jms::{UeMeshPart, UeStaticPart, UeWorldPart};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, LazyLock, Mutex};

const CE_CV: EIoStoreTocVersion = EIoStoreTocVersion::ReplaceIoChunkHashWithIoHash;
const CE_HV: EIoContainerHeaderVersion = EIoContainerHeaderVersion::SoftPackageReferences;

pub(super) fn ensure_model_preview_loaded(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    state: &mut ModelPreviewState,
) {
    if state.loaded_key.as_deref() == Some(entry.key.as_str())
        && state.data.is_some()
        && state.loaded_high_detail == state.high_detail
    {
        return;
    }
    state.loaded_key = Some(entry.key.clone());
    state.loaded_high_detail = state.high_detail;
    let high_detail = state.high_detail;
    state.data = Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            load_model_preview(model_tag, entry, names, source, high_detail)
        }))
        .map_err(|_| "Render model preview crashed while parsing this tag.".to_owned())
        .and_then(|result| result)
        .map(|data| {
            state.render_model_path = Some(data.render_model_path.clone());
            // Auto-select the canonical variant (named `default`, else the first)
            // so the preview opens showing a complete configured model.
            let default_variant = default_variant_index(&data.variants);
            reset_model_preview_selection(state, &data, default_variant);
            data
        }),
    );
}

pub(super) fn load_model_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    high_detail: bool,
) -> Result<ModelPreviewData, String> {
    // Halo CE `gbxmodel` (mod2) and a bare `render_model` (mode) ARE the
    // render geometry — there is no `.model` (hlmt) wrapper carrying a
    // "render model" reference, so preview the tag itself.
    let group = model_tag.header.group_tag.to_be_bytes();
    if matches!(&group, b"mode" | b"mod2") {
        let preview = build_render_preview(model_tag)?;
        if preview.batches.is_empty() {
            return Err("This render tag has no previewable draw batches.".to_owned());
        }
        let max_preview_edge = preview_edge_limit(preview.bounds_min, preview.bounds_max);
        let draw_triangles = build_model_source_triangles(&preview, max_preview_edge);
        return Ok(ModelPreviewData {
            source_key: entry.key.clone(),
            render_model_path: entry.display_path.clone(),
            preview,
            draw_triangles,
            variants: Vec::new(),
        });
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
        key: format!("file:{}", path.display()),
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
    let max_preview_edge = preview_edge_limit(preview.bounds_min, preview.bounds_max);
    let draw_triangles = build_model_source_triangles(&preview, max_preview_edge);
    Ok(ModelPreviewData {
        source_key: render_entry.key,
        render_model_path: normalized,
        preview,
        draw_triangles,
        variants: read_model_variants(model_tag),
    })
}

/// Build preview geometry from a render-geometry tag — a `render_model`
/// (`mode`), a Halo CE `gbxmodel` (`mod2`), or a Halo 2 `render_model`.
///
/// One tag→geometry path for every engine: blam-tags' `RenderModel::from_tag`
/// / `derive_render_meshes` game-dispatch (H3 reads `render geometry`, H2 the
/// `sections`, Halo CE the gbxmodel `geometries`), so batches carry the render
/// model's own region/permutation names and stay in sync with the variant
/// selection. JMS is export-only — never used for rendering.
pub(super) fn build_render_preview(render_tag: &TagFile) -> Result<RenderModelPreview, String> {
    let render_model = RenderModel::from_tag(render_tag).map_err(|error| error.to_string())?;
    let render_meshes =
        RenderModel::derive_render_meshes(render_tag).map_err(|error| error.to_string())?;
    Ok(render_model_to_preview(&render_model, &render_meshes))
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
        model_tag, entry, src, containers, &skel_ref, high_detail,
    ))
}

fn build_campaign_evolved_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    source: &TagSource,
    containers: &[MountedContainer],
    skel_ref: &str,
    high_detail: bool,
) -> Result<ModelPreviewData, String> {
    let _ = model_tag;
    // 1. Resolve the skeleton_model (node skeleton + markers + regions) through
    //    the container tag index — the same read the browser tree performs.
    let skel = source
        .read_container_tag_by_ref(u32::from_be_bytes(*b"skel"), skel_ref)
        .map_err(|e| e.to_string())?;

    // 2. This model's package key, as DA_MeshSynchronization imports it
    //    (e.g. `objects/characters/elite_ai/elite_ai-model`).
    let TagEntryLocation::Container { rel_path, .. } = &entry.location else {
        return Err("CE model preview requires a container entry.".to_owned());
    };
    let stem = rel_path.to_ascii_lowercase().replace('\\', "/");
    let stem = stem.strip_suffix(".ubulk").unwrap_or(&stem);
    let model_key = stem.rsplit("tags/").next().unwrap_or(stem).to_string();

    // 3. Read the hlmt variants (region→permutation): the set of
    //    (region, perm) pairs any variant activates.
    let variants = read_model_variants(model_tag);
    let mut needed: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
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
            (ce_collect_parts_from_regions(containers, &regions, &needed, high_detail), format!("meshsync:{model_key}"),)
        }
        None => (CeMeshes::default(), String::new()),
    };
    let (meshes, render_path) = if meshes.is_empty() {
        let char_root = ce_find_character_root(containers, &model_key).ok_or_else(|| {
            // First-person hand/body models (GameGlobals FirstPersonHands /
            // FirstPersonBody) are a separate representation: no world
            // DA_MeshSynchronization imports them, and their geometry is
            // sourced through the FirstPerson weapon/equipment actors — which
            // this world-model preview doesn't reconstruct.
            if is_first_person_model(&model_key) {
                "First-person hand/body models aren't previewable here — their geometry is \
                 provided by the first-person weapon actors, not the world mesh-sync path."
                    .to_owned()
            } else {
                "No MeshSynchronization data asset references this model — cannot locate its UE meshes."
                    .to_owned()
            }
        })?;
        let skeletal = ce_load_variant_meshes(containers, &char_root, &needed);
        (CeMeshes { skeletal, ..Default::default() }, char_root,)
    } else {
        (meshes, render_path)
    };
    // Human characters' heads come from a separate MetaHuman `Face` component
    // (DT_MetaHumanHeads), not the mesh-sync path — resolve and fuse it in.
    let mut meshes = meshes;
    let head_node = ce_head_node_name(&skel);
    ce_add_metahuman_head(containers, &model_key, &needed, &head_node, &mut meshes);
    if meshes.is_empty() {
        return Err("No UE meshes resolved for this model.".to_owned());
    }
    let parts: Vec<UeMeshPart> = meshes
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
    let static_parts: Vec<UeStaticPart> = meshes
        .statics
        .iter()
        .map(|(region, perm, name, mesh, bone, mats, xf, wa)| UeStaticPart {
            mesh: &**mesh,
            bone_name: bone.clone(),
            region: region.clone(),
            permutation: perm.clone(),
            name: name.clone(),
            material_names: mats.clone(),
            rel_transform: *xf,
            world_anchor: *wa,
        },)
        .collect();
    let world_parts: Vec<UeWorldPart> = meshes
        .world
        .iter()
        .map(|(region, perm, name, mesh, node, mats, anchor)| UeWorldPart {
            mesh: &**mesh,
            node_name: node.clone(),
            head_anchor: *anchor,
            region: region.clone(),
            permutation: perm.clone(),
            name: name.clone(),
            material_names: mats.clone(),
        },)
        .collect();

    // 5. Reconstruct the cross-game RenderModel and run the standard pipeline.
    let (render_model, render_meshes) =
        RenderModel::from_ue_meshes(&parts, &static_parts, &world_parts, &skel).map_err(|e| e.to_string())?;
    if std::env::var("CE_DEBUG").is_ok() {
        let skel_nodes = skel.root().field_path("nodes").and_then(|f| f.as_block()).map(|b| b.len()).unwrap_or(0);
        eprintln!("[CE] skel_ref='{skel_ref}' skel_nodes={skel_nodes}");
        eprintln!("[CE] skeletal parts: {}, static parts: {}", parts.len(), static_parts.len());
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
                (mn[0] + mx[0]) / 2.0, (mn[1] + mx[1]) / 2.0, (mn[2] + mx[2]) / 2.0,
                mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2],
                m.rigid_node_index,
            );
        }
        for r in &render_model.regions {
            for p in &r.permutations {
                eprintln!("[CE] region '{}' perm '{}' idx={} count={}", r.name, p.name, p.mesh_index, p.mesh_count);
            }
        }
    }
    let preview = render_model_to_preview(&render_model, &render_meshes);
    if preview.batches.is_empty() {
        return Err("Reconstructed CE model has no previewable geometry.".to_owned());
    }
    let max_preview_edge = preview_edge_limit(preview.bounds_min, preview.bounds_max);
    let draw_triangles = build_model_source_triangles(&preview, max_preview_edge);
    Ok(ModelPreviewData {
        source_key: entry.key.clone(),
        render_model_path: render_path,
        preview,
        draw_triangles,
        variants,
    })
}

/// Build a full-resolution JMS for a Campaign Evolved `hlmt` model by fusing
/// its Unreal render geometry (skeletal + **Nanite** static pieces) onto the
/// classic `skeleton_model` rig. Mirrors [`build_campaign_evolved_preview`]'s
/// mesh resolution but loads the high-detail Nanite geometry and emits JMS —
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
    let skel = source
        .read_container_tag_by_ref(u32::from_be_bytes(*b"skel"), skel_ref)
        .map_err(|e| e.to_string())?;

    let TagEntryLocation::Container { rel_path, .. } = &entry.location else {
        return Err("CE render extraction requires a container entry.".to_owned());
    };
    let stem = rel_path.to_ascii_lowercase().replace('\\', "/");
    let stem = stem.strip_suffix(".ubulk").unwrap_or(&stem);
    let model_key = stem.rsplit("tags/").next().unwrap_or(stem).to_string();

    let variants = read_model_variants(model_tag);
    let mut needed: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    for v in &variants {
        for (region, perm) in &v.regions {
            if !perm.is_empty() {
                needed.insert((region.to_ascii_lowercase(), perm.to_ascii_lowercase()));
            }
        }
    }

    let meshes = match ce_load_meshsync_regions(containers, &model_key) {
        Some(regions) => ce_collect_parts_from_regions(containers, &regions, &needed, true),
        None => CeMeshes::default(),
    };
    let meshes = if meshes.is_empty() {
        let char_root = ce_find_character_root(containers, &model_key).ok_or_else(||
            "No MeshSynchronization data asset references this model.".to_owned())?;
        CeMeshes {
            skeletal: ce_load_variant_meshes(containers, &char_root, &needed),
            ..Default::default()
        }
    } else {
        meshes
    };
    let mut meshes = meshes;
    let head_node = ce_head_node_name(&skel);
    ce_add_metahuman_head(containers, &model_key, &needed, &head_node, &mut meshes);
    if meshes.is_empty() {
        return Err("No UE meshes resolved for this model.".to_owned());
    }

    let parts: Vec<UeMeshPart> = meshes
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
    let static_parts: Vec<UeStaticPart> = meshes
        .statics
        .iter()
        .map(|(region, perm, name, mesh, bone, mats, xf, wa)| UeStaticPart {
            mesh: &**mesh,
            bone_name: bone.clone(),
            region: region.clone(),
            permutation: perm.clone(),
            name: name.clone(),
            material_names: mats.clone(),
            rel_transform: *xf,
            world_anchor: *wa,
        },)
        .collect();
    let world_parts: Vec<UeWorldPart> = meshes
        .world
        .iter()
        .map(|(region, perm, name, mesh, node, mats, anchor)| UeWorldPart {
            mesh: &**mesh,
            node_name: node.clone(),
            head_anchor: *anchor,
            region: region.clone(),
            permutation: perm.clone(),
            name: name.clone(),
            material_names: mats.clone(),
        },)
        .collect();

    blam_tags::jms::JmsFile::from_ue_meshes(&parts, &static_parts, &world_parts, &skel)
        .map_err(|e| e.to_string())
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
    for c in containers {
        for e in c.archive.entries() {
            let norm = e.path.to_ascii_lowercase().replace('\\', "/");
            if !(norm.ends_with(".uasset") && norm.contains("meshsync")) {
                continue;
            }
            let Ok(bytes) = c.archive.read(&e.path) else { continue; };
            let Ok(hdr) =
                FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None,)
            else {
                continue;
            };
            let hit = hdr.imported_package_names.iter().any(|p| {
                p.to_ascii_lowercase().replace('\\', "/").ends_with(model_key)
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
static CE_CLASSES: LazyLock<(FPackageObjectIndex, FPackageObjectIndex, FPackageObjectIndex,)> =
    LazyLock::new(|| {
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
            let Some(hdr) = ce_read_header(&c.archive, &e.path) else { continue; };
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
                    da_to_models.entry(ce_stem(&e.path)).or_default().extend(models);
                }
            }
            if has_comp {
                let imported_stems: Vec<String> = hdr
                    .imported_package_names
                    .iter()
                    .map(|p| ce_stem(p))
                    .collect();
                actors.push((ActorRef { container: ci, path: e.path.clone(), }, imported_stems,));
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
    let key = containers
        .iter()
        .map(|c| c.utoc_path.display().to_string())
        .collect::<Vec<_>>()
        .join("|");
    let mut cache = CE_INDEX_CACHE.lock().unwrap();
    if let Some(idx) = cache.get(&key) {
        return idx.clone();
    }
    let idx = Arc::new(build_ce_mesh_sync_index(containers));
    cache.insert(key, idx.clone());
    idx
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
    let want = basename.to_ascii_lowercase();
    for c in containers {
        for e in c.archive.entries() {
            let p = e.path.to_ascii_lowercase();
            if !p.ends_with(".uasset") {
                continue;
            }
            let base = p.rsplit('/').next().unwrap_or(&p).trim_end_matches(".uasset");
            if base == want {
                if let Ok(bytes) = c.archive.read(&e.path) {
                    return Some(bytes);
                }
            }
        }
    }
    None
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
) -> Option<Vec<(String, std::collections::BTreeMap<String, PropValue>)>> {
    let mut usmap = Usmap::meteorite().ok()?;
    let sbytes = ce_find_uasset_by_basename(containers, struct_basename)?;
    let shdr = FZenPackageHeader::deserialize(&mut Cursor::new(&sbytes[..]), None, CE_CV, CE_HV, None).ok()?;
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
    let dhdr = FZenPackageHeader::deserialize(&mut Cursor::new(&dbytes[..]), None, CE_CV, CE_HV, None).ok()?;
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
    (!sp.is_empty()).then(|| (sp.package.clone(), sp.asset.clone()))
}

/// Decode `DT_MetaHumanHeads` → per-row face + facial-hair mesh references.
fn ce_decode_head_table(containers: &[MountedContainer],) -> Option<Vec<(String, MetaHumanHeadRow)>> {
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
                    row.hair = arr.iter().filter_map(PropValue::as_soft_object).filter_map(ce_soft).collect();
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
fn ce_decode_helmet_table(containers: &[MountedContainer],) -> Option<Vec<(String, MetaHumanHelmetRow)>> {
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
                if let Some(m) = fields.get("Mesh").and_then(PropValue::as_soft_object).and_then(ce_soft) {
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
        let Some(hdr) = ce_read_header(&c.archive, &r.path) else { continue; };
        let is_human = hdr
            .imported_package_names
            .iter()
            .any(|p| { p.rsplit('/').next().unwrap_or(p).eq_ignore_ascii_case(CE_MH_COMPONENT)
        });
        if !is_human {
            continue;
        }
        let base = r.path.rsplit('/').next().unwrap_or(&r.path);
        let base = base.strip_suffix(".uasset").unwrap_or(base).to_ascii_lowercase();
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
        if let Some(n) = names.iter().find(|n| n.eq_ignore_ascii_case("head_m") || n.eq_ignore_ascii_case("head")) {
            return n.clone();
        }
        if let Some(n) = names.iter().find(|n| n.to_ascii_lowercase().contains("head")) {
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
    out: &mut CeMeshes,
) {
    let Some(key) = ce_metahuman_character_key(containers, model_key) else { return; };
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
        let female = key.contains("female")
            || model_key.to_ascii_lowercase().contains("female");
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
            let Some(mesh) = ce_read_skeletal_mesh(containers, pkg) else { continue; };
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
            } else if let Some(mesh) = ce_read_static_mesh(containers, pkg) {
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
        .or_else(|| { face.bones.iter().position(|b| b.name.to_ascii_lowercase().contains("head"))
        })?;
    let world = blam_tags::jms::ue_bind_world(&face.bones);
    let m = world.get(idx)?;
    Some([m.m[0][3], m.m[1][3], m.m[2][3]])
}

/// Read + decode a `USkeletalMesh` by package path (`None` on any failure).
fn ce_read_skeletal_mesh(containers: &[MountedContainer], pkg: &str) -> Option<Arc<SkeletalMesh>> {
    let (_, bytes) = ce_read_uasset_by_package(containers, pkg)?;
    let hdr = FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None).ok()?;
    let names = hdr.name_map.copy_raw_names();
    SkeletalMesh::from_package(&bytes, &names, hdr.summary.header_size as usize).ok().map(Arc::new)
}

/// Read + decode a `UStaticMesh` (coarse LOD, sufficient for a hat) by package.
fn ce_read_static_mesh(containers: &[MountedContainer], pkg: &str) -> Option<Arc<StaticMesh>> {
    let (_, bytes) = ce_read_uasset_by_package(containers, pkg)?;
    let hdr = FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None).ok()?;
    StaticMesh::from_package(&bytes, hdr.summary.header_size as usize).ok().map(Arc::new)
}

/// The default per-slot materials of a mesh package (its `MI_`/`M_` imports).
fn ce_read_default_materials(containers: &[MountedContainer], pkg: &str) -> Vec<String> {
    let Some((_, bytes)) = ce_read_uasset_by_package(containers, pkg) else { return Vec::new(); };
    let Ok(hdr) = FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None) else {
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
        let Some(c) = containers.get(r.container) else { continue; };
        let Ok(bytes) = c.archive.read(&r.path) else { continue; };
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
        let Some(export) = bytes.get(start..end) else { continue; };
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
        let ri = match dst.regions.iter().position(|r| r.name.eq_ignore_ascii_case(&sr.name)) {
            Some(i) => i,
            None => {
                dst.regions.push(Region { name: sr.name.clone(), permutations: Vec::new(), });
                dst.regions.len() - 1
            }
        };
        for sp in sr.permutations {
            let perms = &mut dst.regions[ri].permutations;
            let pi = match perms.iter().position(|p| p.name.eq_ignore_ascii_case(&sp.name)) {
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
    skeletal: Vec<(String, String, String, std::sync::Arc<SkeletalMesh>, Vec<String>,)>,
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
    world: Vec<(String, String, String, std::sync::Arc<SkeletalMesh>, String, Vec<String>, [f32; 3],)>,
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
fn ce_effective_materials(default_materials: &[String], overrides: &[(String, String)],) -> Vec<String> {
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
                let hdr =
                    FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None,)
                        .ok()?;
                let names = hdr.name_map.copy_raw_names();
                let mesh = SkeletalMesh::from_package(&bytes, &names, hdr.summary.header_size as usize).ok()?;
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
                let hdr =
                    FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CE_CV, CE_HV, None,)
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

/// Read a `.uasset` by its UE package path (`/Game/Characters/.../SK_Foo`),
/// matching the container entry whose path ends with the corresponding
/// `Content/...SK_Foo.uasset` tail.
fn ce_read_uasset_by_package(
    containers: &[MountedContainer],
    package: &str,
) -> Option<(String, Vec<u8>)> {
    let tail = package.to_ascii_lowercase().replace('\\', "/");
    let tail = tail.strip_prefix("/game/").unwrap_or(&tail);
    let suffix = format!("/{tail}.uasset");
    for c in containers {
        for e in c.archive.entries() {
            if e.path.to_ascii_lowercase().replace('\\', "/").ends_with(&suffix) {
                if let Ok(bytes) = c.archive.read(&e.path) {
                    return Some((e.path.clone(), bytes));
                }
            }
        }
    }
    None
}

/// Read the sibling `.ubulk` (Nanite streaming pages) for a package, matched
/// the same way as [`ce_read_uasset_by_package`]. Bulk data isn't in the
/// directory index — it shares the package's chunk id with the BulkData type,
/// fetched via [`IoStoreArchive::read_bulk_for`].
fn ce_read_bulk_by_package(containers: &[MountedContainer], package: &str) -> Option<Vec<u8>> {
    let tail = package.to_ascii_lowercase().replace('\\', "/");
    let tail = tail.strip_prefix("/game/").unwrap_or(&tail);
    let suffix = format!("/{tail}.uasset");
    for c in containers {
        for e in c.archive.entries() {
            if e.path.to_ascii_lowercase().replace('\\', "/").ends_with(&suffix) {
                return c.archive.read_bulk_for(e.chunk_index, 0).ok();
            }
        }
    }
    None
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
) -> Vec<(String, String, String, std::sync::Arc<SkeletalMesh>, Vec<String>,)> {
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
        let Some(stem) = resolve(region, perm) else { continue; };
        let Some((ci, path)) = sk.get(&stem).cloned() else { continue; };
        let Ok(bytes) = containers[ci].archive.read(&path) else { continue; };
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
        out.push((region.clone(), perm.clone(), stem, std::sync::Arc::new(mesh), mats,));
    }
    out
}

/// Non-renderable / overlay meshes to skip (skeleton bind mesh, shield/shadow
/// proxies, cloth AnimDynamics, damage states, collision/physics/imposters).
fn ce_is_excluded(norm: &str, base: &str) -> bool {
    norm.contains("/skeleton/")
        || [
            "shield", "shadow", "animdynamics", "destroyed", "_dmg", "damage", "collision",
            "physics", "imposter",
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

    /// Runs the EXACT app CE-preview path (real mounted source) and dumps where
    /// each reconstructed mesh actually bakes — to isolate the wing/helmet
    /// placement issue from render-side effects. Skips if the CE paks aren't
    /// present on this machine.
    #[test]
    fn ce_pelican_real_path() {
        let paks = PathBuf::from(
            "/Users/camden/Halo/halo-campaign-evolved_pc/Meteorite/Content/Paks"
        );
        if !paks.exists() {
            eprintln!("skip: CE paks not found");
            return;
        }
        let defs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let loaded = crate::source::load_iostore_container_set(
            paks,
            &TagNameIndex::default(),
            &defs
        )
        .expect("mount CE container set");
        let source = &loaded.source;
        let hlmt = u32::from_be_bytes(*b"hlmt");
        let entry = loaded
            .entries
            .iter()
            .chain(loaded.all_entries.iter())
            .find(|e| {
                e.group_tag == hlmt
                    && e.display_path.to_ascii_lowercase().contains(&std::env::var("CE_MODEL").unwrap_or_else(|_|"pelican/pelican".into()),)
            })
            .expect("pelican.model entry")
            .clone();
        eprintln!("[TEST] entry: {} loc={:?}", entry.display_path, std::mem::discriminant(&entry.location));
        let tag = read_entry(source, &entry).expect("read pelican.model");
        unsafe { std::env::set_var("CE_DEBUG", "1"); }
        let data = load_campaign_evolved_preview(&tag, &entry, Some(source), std::env::var("CE_HD").is_ok(),)
            .expect("recognized as CE model")
            .expect("CE preview built");
        eprintln!(
            "[TEST] preview bounds min{:?} max{:?}  ({} draw tris)",
            data.preview.bounds_min, data.preview.bounds_max, data.draw_triangles.len()
        );
        // Optional OBJ export of the exact preview geometry (set CE_OBJ).
        if std::env::var("CE_OBJ").is_ok() {
            use std::io::Write;
            let out = format!(
                "/private/tmp/claude-501/-Users-camden-Source-Baboon-local/4803b682-de10-4887-907a-9f81ad3d13d0/scratchpad/{}.obj",
                std::env::var("CE_MODEL").unwrap_or_else(|_| "pelican".into()).replace('/', "_")
            );
            let mut f = std::fs::File::create(&out).unwrap();
            for t in &data.draw_triangles {
                for pos in &t.positions {
                    writeln!(f, "v {} {} {}", pos[0], pos[1], pos[2]).unwrap();
                }
            }
            for i in 0..data.draw_triangles.len() {
                let b = (i * 3 + 1) as u32;
                writeln!(f, "f {} {} {}", b, b + 1, b + 2).unwrap();
            }
            eprintln!("[TEST] wrote {} ({} tris)", out, data.draw_triangles.len());
        }
    }

}
