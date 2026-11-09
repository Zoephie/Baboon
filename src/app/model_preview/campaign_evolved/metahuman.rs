//! MetaHuman heads and helmets: their tables and the head added to a preview.

use super::*;

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
    let key = ce_container_set_key(containers);
    let mut cache = CE_MH_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

/// The first export's serial byte slice within a package.
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
pub(super) fn ce_head_node_name(skel: &TagFile) -> String {
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

pub(super) fn ce_add_metahuman_head(
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
