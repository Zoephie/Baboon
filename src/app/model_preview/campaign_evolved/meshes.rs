//! A model's meshes: skeletal and static meshes, their regions, materials and parts.

use super::super::super::{TagEntry, TagEntryLocation};
use super::super::{ModelVariantPreview, TagReader, read_model_variants};
use super::MountedContainer;
use super::export::{CEV_CV, CEV_HV};
use super::index::{
    ActorRef, CEV_CLASSES, cev_find_character_root, cev_mesh_sync_index, is_first_person_model,
};
use super::metahuman::{cev_add_metahuman_head, cev_head_node_name};
use super::packages::{
    cev_load_variant_meshes, cev_read_bulk_by_package, cev_read_uasset_by_package,
};
use blam_tags::TagFile;
use blam_tags::iostore::skeletal_mesh::SkeletalMesh;
use blam_tags::iostore::static_mesh::StaticMesh;
use blam_tags::iostore::unversioned::{MeshRef, MeshSyncRegions, Permutation, Region};
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::zen::FZenPackageHeader;
use blam_tags::jms::{UeMeshPart, UeStaticPart, UeWorldPart};
use std::io::Cursor;
use std::sync::Arc;

/// A Campaign Evolved model's skeleton and Unreal render geometry, resolved
/// the one way both its preview and its JMS export need.
pub(super) struct CevModelMeshes {
    pub(super) skel: TagFile,
    pub(super) variants: Vec<ModelVariantPreview>,
    pub(super) meshes: CevMeshes,
    /// Where the geometry came from, for the preview's label.
    pub(super) render_path: String,
}

pub(super) fn resolve_ce_model_meshes(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: &TagReader,
    containers: &[MountedContainer],
    skel_ref: &str,
    high_detail: bool,
) -> Result<CevModelMeshes, String> {
    // 1. Resolve the skeleton_model (node skeleton + markers + regions) through
    //    the container tag index — the same read the browser tree performs.
    let skel = reader
        .source
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
    let (meshes, render_path) = match cev_load_meshsync_regions(containers, &model_key) {
        Some(regions) => {
            // `high_detail` decodes the full-resolution Nanite geometry (slow,
            // millions of tris); otherwise the coarse LOD fallback (fast).
            (
                cev_collect_parts_from_regions(containers, &regions, &needed, high_detail),
                format!("meshsync:{model_key}"),
            )
        }
        None => (CevMeshes::default(), String::new()),
    };
    let (mut meshes, render_path) = if meshes.is_empty() {
        let char_root = cev_find_character_root(containers, &model_key).ok_or_else(|| {
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
        let skeletal = cev_load_variant_meshes(containers, &char_root, &needed);
        (
            CevMeshes {
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
    let head_node = cev_head_node_name(&skel);
    cev_add_metahuman_head(
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
    Ok(CevModelMeshes {
        skel,
        variants,
        meshes,
        render_path,
    })
}

/// The resolved meshes as the part lists the cross-game builders take.
pub(super) fn cev_mesh_parts(
    meshes: &CevMeshes,
) -> (
    Vec<UeMeshPart<'_>>,
    Vec<UeStaticPart<'_>>,
    Vec<UeWorldPart<'_>>,
) {
    let parts = meshes
        .skeletal
        .iter()
        .map(|(region, perm, name, mesh, mats)| UeMeshPart {
            mesh,
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
                mesh,
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
                mesh,
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

/// Read + decode a `USkeletalMesh` by package path (`None` on any failure).
pub(super) fn cev_read_skeletal_mesh(
    containers: &[MountedContainer],
    pkg: &str,
) -> Option<Arc<SkeletalMesh>> {
    let (_, bytes) = cev_read_uasset_by_package(containers, pkg)?;
    let hdr =
        FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CEV_CV, CEV_HV, None)
            .ok()?;
    let names = hdr.name_map.copy_raw_names();
    SkeletalMesh::from_package(&bytes, &names, hdr.summary.header_size as usize)
        .ok()
        .map(Arc::new)
}

/// Read + decode a `UStaticMesh` by package, preferring Nanite bulk data when
/// `high_detail` is enabled and falling back to the coarse cooked LOD.
pub(super) fn cev_read_static_mesh(
    containers: &[MountedContainer],
    pkg: &str,
    high_detail: bool,
) -> Option<Arc<StaticMesh>> {
    let (_, bytes) = cev_read_uasset_by_package(containers, pkg)?;
    let hdr =
        FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CEV_CV, CEV_HV, None)
            .ok()?;
    let bulk = high_detail
        .then(|| cev_read_bulk_by_package(containers, pkg))
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
pub(super) fn cev_read_default_materials(
    containers: &[MountedContainer],
    pkg: &str,
) -> Vec<String> {
    let Some((_, bytes)) = cev_read_uasset_by_package(containers, pkg) else {
        return Vec::new();
    };
    let Ok(hdr) =
        FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CEV_CV, CEV_HV, None)
    else {
        return Vec::new();
    };
    cev_default_materials(&hdr)
}

/// Decode the authoritative region→permutation→mesh mapping for a model from
/// the actor Blueprint(s) that render it. Chain: this model's package is
/// imported by a `BlamMeshSynchronizationDataAsset` (identified by class), which
/// is imported by the actor Blueprint whose `BlamMeshSynchronizationComponent`
/// (also by class) bakes the `RuntimeRegions` map. Actors and their bindings are
/// resolved once into a cached [`CevMeshSyncIndex`]; this just decodes the few
/// relevant actors' world regions and merges them.
fn cev_load_meshsync_regions(
    containers: &[MountedContainer],
    model_key: &str,
) -> Option<MeshSyncRegions> {
    let index = cev_mesh_sync_index(containers);
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

    let (_, comp_class, comp_base_class) = *CEV_CLASSES;
    let usmap = Usmap::meteorite().ok()?;
    let mut merged = MeshSyncRegions::default();
    for r in refs {
        let Some(c) = containers.get(r.container) else {
            continue;
        };
        let Ok(bytes) = c.archive.read(&r.path) else {
            continue;
        };
        let Ok(hdr) = FZenPackageHeader::deserialize(
            &mut Cursor::new(&bytes[..]),
            None,
            CEV_CV,
            CEV_HV,
            None,
        ) else {
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
                cev_merge_regions(&mut merged, regions);
            }
        }
    }
    (!merged.regions.is_empty()).then_some(merged)
}

/// Merge one actor's world `RuntimeRegions` into the accumulator, deduping
/// meshes by `(package, parent_bone)` so overlapping actors (or world variants
/// of the same actor) don't double-emit the same piece.
fn cev_merge_regions(dst: &mut MeshSyncRegions, src: MeshSyncRegions) {
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
pub(super) struct CevMeshes {
    /// `(region, perm, asset name, mesh, material names)`. Meshes are shared
    /// (`Arc`) because color variants reference the same geometry — loading and
    /// (Nanite-)decoding each package once, not once per variant.
    pub(super) skeletal: Vec<(
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
    pub(super) statics: Vec<(
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
    pub(super) world: Vec<(
        String,
        String,
        String,
        std::sync::Arc<SkeletalMesh>,
        String,
        Vec<String>,
        [f32; 3],
    )>,
}

impl CevMeshes {
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
fn cev_default_materials(hdr: &FZenPackageHeader) -> Vec<String> {
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
fn cev_effective_materials(
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
fn cev_collect_parts_from_regions(
    containers: &[MountedContainer],
    regions: &MeshSyncRegions,
    needed: &std::collections::BTreeSet<(String, String)>,
    nanite: bool,
) -> CevMeshes {
    let mut out = CevMeshes::default();
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
                let (_, bytes) = cev_read_uasset_by_package(containers, &mref.package)?;
                let hdr = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&bytes[..]),
                    None,
                    CEV_CV,
                    CEV_HV,
                    None,
                )
                .ok()?;
                let names = hdr.name_map.copy_raw_names();
                let mesh =
                    SkeletalMesh::from_package(&bytes, &names, hdr.summary.header_size as usize)
                        .ok()?;
                Some((std::sync::Arc::new(mesh), cev_default_materials(&hdr)))
            });
            if let Some((mesh, default_mats)) = entry {
                out.skeletal.push((
                    region.clone(),
                    perm.clone(),
                    mref.asset.clone(),
                    mesh.clone(),
                    cev_effective_materials(default_mats, &mref.material_overrides),
                ));
            }
        }
        for mref in regions.static_meshes(region, perm) {
            let entry = sm_cache.entry(mref.package.clone()).or_insert_with(|| {
                let (_, bytes) = cev_read_uasset_by_package(containers, &mref.package)?;
                let hdr = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&bytes[..]),
                    None,
                    CEV_CV,
                    CEV_HV,
                    None,
                )
                .ok()?;
                // For extraction (and the preview's high-detail mode) prefer the
                // full-resolution Nanite geometry from the package's `.ubulk`;
                // the light preview uses the coarse LOD fallback.
                let ubulk = if nanite {
                    cev_read_bulk_by_package(containers, &mref.package)
                } else {
                    None
                };
                let mesh = StaticMesh::from_package_preferring_nanite(
                    &bytes,
                    hdr.summary.header_size as usize,
                    ubulk.as_deref(),
                )
                .ok()?;
                Some((std::sync::Arc::new(mesh), cev_default_materials(&hdr)))
            });
            if let Some((mesh, default_mats)) = entry {
                out.statics.push((
                    region.clone(),
                    perm.clone(),
                    mref.asset.clone(),
                    mesh.clone(),
                    mref.parent_bone.clone(),
                    cev_effective_materials(default_mats, &mref.material_overrides),
                    mref.rel_transform,
                    None,
                ));
            }
        }
    }
    out
}
