//! Reading a package's assets and its variant meshes.

use super::*;

/// The `.uasset` entries of a UE package path (`/Game/Characters/.../SK_Foo`):
/// those whose path ends with the corresponding `/...SK_Foo.uasset` tail, in
/// the order a scan of every container meets them.
///
/// Answered from the file-name index: a path with that tail has the tail's
/// last segment as its file name, so only those few entries are compared.
pub(super) fn cev_package_entries<'c>(
    containers: &'c [MountedContainer],
    package: &str,
) -> impl Iterator<Item = (&'c MountedContainer, &'c blam_tags::iostore::Entry)> {
    let (file_name, suffix) = cev_package_file_name_and_suffix(package);
    let candidates = cev_path_index(containers)
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
pub(super) fn cev_package_file_name_and_suffix(package: &str) -> (String, String) {
    let tail = package.to_ascii_lowercase().replace('\\', "/");
    let tail = tail.strip_prefix("/game/").unwrap_or(&tail);
    let file_name = format!("{}.uasset", tail.rsplit('/').next().unwrap_or(tail));
    (file_name, format!("/{tail}.uasset"))
}

/// Read a `.uasset` by its UE package path; see [`cev_package_entries`].
pub(super) fn cev_read_uasset_by_package(
    containers: &[MountedContainer],
    package: &str,
) -> Option<(String, Vec<u8>)> {
    cev_package_entries(containers, package).find_map(|(c, e)| {
        c.archive
            .read(&e.path)
            .ok()
            .map(|bytes| (e.path.clone(), bytes))
    })
}

/// Read the sibling `.ubulk` (Nanite streaming pages) for a package, matched
/// the same way as [`cev_read_uasset_by_package`]. Bulk data isn't in the
/// directory index — it shares the package's chunk id with the BulkData type,
/// fetched via [`IoStoreArchive::read_bulk_for`].
pub(super) fn cev_read_bulk_by_package(containers: &[MountedContainer], package: &str) -> Option<Vec<u8>> {
    let (c, e) = cev_package_entries(containers, package).next()?;
    c.archive.read_bulk_for(e.chunk_index, 0).ok()
}

/// Variant-driven mesh loading: for each `(region, permutation)` the hlmt's
/// variants reference, locate the exact UE `SK_` mesh by name and load it,
/// bound to that authoritative region/permutation. The mesh naming encodes
/// the permutation (`SK_Marine_Torso_01` = region `body`, perm `torso_01`);
/// the head's `default` face/skin lives in the character's `anatomy` mesh.
/// Meshes the game keeps as `SM_` static (helmets/armor) or MetaHuman (faces)
/// aren't `SK_` and simply don't resolve here (a known gap).
pub(super) fn cev_load_variant_meshes(
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
            if !base.starts_with("sk_") || cev_is_excluded(&norm, base) || base.contains("female") {
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
            FZenPackageHeader::deserialize(&mut Cursor::new(&bytes[..]), None, CEV_CV, CEV_HV, None)
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
fn cev_is_excluded(norm: &str, base: &str) -> bool {
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
