use crate::core::source::{load_iostore_container_set, read_entry};
use std::path::{Path, PathBuf};

static PAKS: std::sync::LazyLock<&'static str> =
    std::sync::LazyLock::new(|| crate::test_kits::leak(crate::test_kits::ce_paks()));

/// Mirrors how the editor builds paths: the inherited-parent chain
/// contributes a name-only prefix, leaves add `name#ordinal`.
fn collect(st: blam_tags::TagStruct<'_>, prefix: &str, out: &mut Vec<String>, depth: usize) {
    if depth > 6 {
        return;
    }
    for (chain_struct, chain_prefix) in crate::app::editor::fields::inherited_struct_chain(st) {
        let base = if chain_prefix.is_empty() {
            prefix.to_string()
        } else if prefix.is_empty() {
            chain_prefix.clone()
        } else {
            format!("{prefix}/{chain_prefix}")
        };
        for field in chain_struct.fields() {
            if crate::app::editor::fields::is_inherited_parent_name(field.name()) {
                continue;
            }
            let path = crate::app::editor::append_field_path_for(&base, &field);
            if let Some(block) = field.as_block() {
                if let Some(child) = block.element(0) {
                    collect(child, &format!("{path}[0]"), out, depth + 1);
                }
            } else if let Some(child) = field.as_struct() {
                collect(child, &path, out, depth + 1);
            } else if field.value().is_some() {
                out.push(path);
            }
        }
    }
}

#[test]
fn every_ui_field_path_resolves_on_campaign_evolved_vehicles() {
    if !Path::new(*PAKS).exists() {
        eprintln!("skipping: {} not present", *PAKS);
        return;
    }
    let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
    let loaded =
        load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs).expect("mount");
    let vehicles: Vec<_> = loaded
        .entries
        .iter()
        .filter(|e| e.display_path.ends_with(".vehicle"))
        .cloned()
        .collect();
    eprintln!("{} vehicle tags", vehicles.len());

    let mut total = 0usize;
    let mut broken: Vec<(String, String)> = Vec::new();
    for entry in vehicles.iter().take(8) {
        let Ok(mut tag) = read_entry(&loaded.source, entry) else {
            continue;
        };
        let mut paths = Vec::new();
        collect(tag.root(), "", &mut paths, 0);
        total += paths.len();
        for path in &paths {
            let mut root = tag.root_mut();
            if root.field_path_mut(path).is_none() {
                broken.push((entry.display_path.clone(), path.clone()));
            }
        }
    }
    eprintln!("checked {total} paths; UNRESOLVABLE {}", broken.len());
    for (tagname, p) in broken.iter().take(30) {
        eprintln!("  {tagname}  ->  {p}");
    }
    assert!(broken.is_empty(), "{} unresolvable paths", broken.len());
}
