//! Extract Geometry / Extract Animations for another game's tools.
//!
//! Each test extracts a shipped tag through the entry point the target
//! window calls and reads back what was written: the version each tool
//! accepts, and — across the Halo CE boundary — every triangle still there,
//! in one file per permutation for Halo CE or permutation-and-region material
//! lines for the later tools. Kit-gated: set `BLAM_TEST_HCEEK` /
//! `BLAM_TEST_H2EK` to the kits' `tags` folders.

use std::path::{Path, PathBuf};

use blam_tags::JmsFile;
use blam_tags::game::Game;

use crate::app::export::{extract_animations_for_entry, extract_geometry_for_entry};
use crate::core::source::{TagEntry, TagEntryLocation, TagSource};

fn kit_tag(root: PathBuf, rel: &str) -> Option<PathBuf> {
    if root.join(rel).is_file() {
        Some(root)
    } else {
        eprintln!("skipping: {rel} not present under {}", root.display());
        None
    }
}

fn loose_source(root: &Path, game: &str) -> TagSource {
    TagSource::LooseFolder {
        root: root.to_path_buf(),
        game: Some(game.to_owned()),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    }
}

fn entry_for(root: &Path, rel: &str, group: &[u8; 4]) -> TagEntry {
    let path = root.join(rel);
    TagEntry {
        key: format!("file:{}", path.display()),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*group),
        group_name: Some(
            match group {
                b"mod2" => "gbxmodel",
                b"mode" => "render_model",
                b"vehi" => "vehicle",
                b"jmad" => "model_animation_graph",
                other => panic!("no group name for {other:?}"),
            }
            .to_owned(),
        ),
        location: TagEntryLocation::LooseFile(path),
    }
}

fn fresh_dir(name: &str) -> PathBuf {
    let out = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("create output dir");
    out
}

fn files_with_extension(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(extension))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// A Halo CE (8200) JMS, read section by section in the order Halo CE
/// tool.exe reads it (`sub_4372F0`): it has to come out exactly at the end.
struct Halo1Jms {
    nodes: usize,
    /// Every region, in order; a permutation's file lists the regions it has
    /// no geometry in too.
    regions: Vec<String>,
    /// The regions its triangles use.
    used_regions: std::collections::BTreeSet<String>,
    triangles: usize,
}

fn read_halo1_jms(path: &Path) -> Halo1Jms {
    let text = std::fs::read_to_string(path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "8200", "{}: version", path.display());
    let mut at = 2; // version, checksum
    let count = |at: &mut usize| -> usize {
        let n = lines[*at]
            .parse()
            .unwrap_or_else(|_| panic!("{}: expected a count at line {}", path.display(), *at + 1));
        *at += 1;
        n
    };
    let nodes = count(&mut at);
    at += nodes * 5;
    let materials = count(&mut at);
    at += materials * 2;
    let markers = count(&mut at);
    at += markers * 6;
    let region_count = count(&mut at);
    let regions: Vec<String> = lines[at..at + region_count]
        .iter()
        .map(|r| r.to_string())
        .collect();
    at += region_count;
    let vertices = count(&mut at);
    at += vertices * 7;
    let triangles = count(&mut at);
    let used_regions = (0..triangles)
        .map(|t| regions[lines[at + t * 3].parse::<usize>().unwrap()].clone())
        .collect();
    at += triangles * 3;
    assert_eq!(at, lines.len(), "{}: trailing lines", path.display());
    Halo1Jms {
        used_regions,
        nodes,
        regions,
        triangles,
    }
}

const CE_WARTHOG: &str = "vehicles/warthog/warthog.gbxmodel";
const H2_ELITE: &str = "objects/characters/elite/elite.render_model";

/// Halo CE's warthog for its own tools: one 8200 file per permutation under
/// `models/`, the layout Halo CE tool.exe imports (it names a permutation
/// after its file). Before, every permutation went into one file and came
/// back as one.
#[test]
fn halo1_geometry_for_halo1_is_one_file_per_permutation() {
    let Some(root) = kit_tag(crate::test_kits::hceek_tags(), CE_WARTHOG) else {
        return;
    };
    let out = fresh_dir("baboon_extract_target_ce_ce");
    let message = extract_geometry_for_entry(
        &loose_source(&root, "haloce_mcc"),
        &entry_for(&root, CE_WARTHOG, b"mod2"),
        &out,
        Game::Halo1,
    )
    .expect("extract");
    let files = files_with_extension(&out.join("models"), "jms");
    assert!(files.len() > 1, "expected several permutations: {message}");
    for file in &files {
        let jms = read_halo1_jms(file);
        assert!(jms.nodes > 0 && jms.triangles > 0 && !jms.regions.is_empty());
    }
}

/// The same warthog for Halo 2 and Halo 3's tools: one modern file at each
/// version, every triangle of every permutation in it, with material lines
/// naming the permutation and region each came from.
#[test]
fn halo1_geometry_for_later_tools_merges_permutations_into_labels() {
    let Some(root) = kit_tag(crate::test_kits::hceek_tags(), CE_WARTHOG) else {
        return;
    };
    let source = loose_source(&root, "haloce_mcc");
    let entry = entry_for(&root, CE_WARTHOG, b"mod2");
    let halo1 = fresh_dir("baboon_extract_target_ce_ce_count");
    extract_geometry_for_entry(&source, &entry, &halo1, Game::Halo1).expect("extract for CE");
    let per_permutation: Vec<(String, Halo1Jms)> =
        files_with_extension(&halo1.join("models"), "jms")
            .iter()
            .map(|f| {
                (
                    f.file_stem().unwrap().to_string_lossy().into_owned(),
                    read_halo1_jms(f),
                )
            })
            .collect();
    let triangles: usize = per_permutation.iter().map(|(_, j)| j.triangles).sum();

    for (target, version) in [(Game::Halo2, 8210), (Game::Halo3, 8213)] {
        let out = fresh_dir(&format!("baboon_extract_target_ce_{version}"));
        extract_geometry_for_entry(&source, &entry, &out, target).expect("extract");
        let path = out.join("warthog.render.jms");
        let (jms, read_version) =
            JmsFile::parse(&std::fs::read_to_string(&path).unwrap()).expect("parse");
        assert_eq!(read_version, version);
        assert_eq!(jms.triangles.len(), triangles, "triangles lost merging");
        for (permutation, file) in &per_permutation {
            for region in &file.used_regions {
                let region = region.split_whitespace().collect::<Vec<_>>().join("_");
                assert!(
                    jms.materials.iter().any(|m| {
                        let label = blam_tags::jms_split::MaterialLabel::parse(&m.material_name);
                        label.permutation == *permutation && label.region == region
                    }),
                    "no material line for {permutation} {region}"
                );
            }
        }
    }
}

/// Halo 2's elite for Halo CE's tools: split by the permutation in each
/// material line into 8200 files, with no triangle lost and none duplicated.
#[test]
fn later_geometry_for_halo1_splits_by_permutation() {
    let Some(root) = kit_tag(crate::test_kits::h2ek_tags(), H2_ELITE) else {
        return;
    };
    let source = loose_source(&root, "halo2_mcc");
    let entry = entry_for(&root, H2_ELITE, b"mode");
    let own = fresh_dir("baboon_extract_target_h2_h2");
    extract_geometry_for_entry(&source, &entry, &own, Game::Halo2).expect("extract for H2");
    let (modern, version) =
        JmsFile::parse(&std::fs::read_to_string(own.join("elite.render.jms")).unwrap())
            .expect("parse");
    assert_eq!(version, 8210);
    let permutations: std::collections::BTreeSet<String> = modern
        .materials
        .iter()
        .map(|m| {
            blam_tags::jms_split::MaterialLabel::parse(&m.material_name)
                .permutation
                .to_ascii_lowercase()
        })
        .collect();

    let out = fresh_dir("baboon_extract_target_h2_ce");
    let message = extract_geometry_for_entry(&source, &entry, &out, Game::Halo1).expect("extract");
    let files = files_with_extension(&out.join("models"), "jms");
    let written: std::collections::BTreeSet<String> = files
        .iter()
        .map(|f| {
            f.file_stem()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase()
        })
        .collect();
    assert_eq!(written, permutations, "{message}");
    let triangles: usize = files.iter().map(|f| read_halo1_jms(f).triangles).sum();
    assert_eq!(triangles, modern.triangles.len());
}

/// Animations take the target's JMA version: 16392 for Halo CE, 16394 —
/// with the node checksum second and parent links — for Halo 2 on, whatever
/// game the graph came from.
#[test]
fn animations_take_the_target_jma_version() {
    // Through the object tag: this graph has no nodes of its own and takes
    // them from the vehicle's gbxmodel.
    let ce = "vehicles/warthog/warthog.vehicle";
    let h2 = "objects/characters/elite/elite.model_animation_graph";
    let cases = [
        (crate::test_kits::hceek_tags(), "haloce_mcc", ce, b"vehi"),
        (crate::test_kits::h2ek_tags(), "halo2_mcc", h2, b"jmad"),
    ];
    for (root, game, rel, group) in cases {
        let Some(root) = kit_tag(root, rel) else {
            continue;
        };
        for (target, version) in [
            (Game::Halo1, "16392"),
            (Game::Halo2, "16394"),
            (Game::Halo3, "16394"),
        ] {
            let out = fresh_dir(&format!("baboon_extract_target_anim_{game}_{version}"));
            extract_animations_for_entry(
                &loose_source(&root, game),
                &entry_for(&root, rel, group),
                &out,
                target,
            )
            .expect("extract animations");
            let files: Vec<PathBuf> = ["jmm", "jma", "jmt", "jmz", "jmo", "jmr", "jmw"]
                .iter()
                .flat_map(|ext| files_with_extension(&out, ext))
                .collect();
            assert!(!files.is_empty(), "{game} → {target:?}: nothing written");
            for file in files {
                let text = std::fs::read_to_string(&file).unwrap();
                let lines: Vec<&str> = text.lines().collect();
                assert_eq!(lines[0], version, "{}", file.display());
                if version == "16394" {
                    // Node count is line 7; the first node's parent follows its name.
                    let nodes: usize = lines[6].parse().unwrap();
                    assert!(nodes > 0);
                    assert_eq!(lines[8], "-1", "{}: the root node's parent", file.display());
                }
            }
        }
    }
}
