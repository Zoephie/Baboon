//! G-load: each object of [`LOAD_SET`] loads to what it did before.

use super::*;

/// The objects each game loads: a biped, weapons and vehicles, chosen to reach
/// as many of the preview's features as one load can.
pub(in crate::app::model_preview) const LOAD_SET: &[(&str, GameId, &[(&str, &str, &[u8; 4])])] = &[
    (
        "BLAM_TEST_HCEEK",
        GameId::HaloCe,
        &[
            ("characters\\cyborg\\cyborg", "biped", b"bipd"),
            ("vehicles\\warthog\\warthog", "vehicle", b"vehi"),
            ("vehicles\\banshee\\banshee", "vehicle", b"vehi"),
            ("weapons\\plasma rifle\\plasma rifle", "weapon", b"weap"),
            ("weapons\\assault rifle\\assault rifle", "weapon", b"weap"),
        ],
    ),
    (
        "BLAM_TEST_H2EK",
        GameId::Halo2,
        &[
            (
                "objects\\characters\\masterchief\\masterchief",
                "biped",
                b"bipd",
            ),
            ("objects\\characters\\elite\\elite", "biped", b"bipd"),
            (
                "objects\\weapons\\rifle\\plasma_rifle\\plasma_rifle",
                "weapon",
                b"weap",
            ),
            ("objects\\vehicles\\warthog\\warthog", "vehicle", b"vehi"),
            ("objects\\vehicles\\banshee\\banshee", "vehicle", b"vehi"),
        ],
    ),
    (
        "BLAM_TEST_H3EK",
        GameId::Halo3,
        &[
            (
                "objects\\characters\\masterchief\\masterchief",
                "biped",
                b"bipd",
            ),
            ("objects\\characters\\cortana\\cortana", "biped", b"bipd"),
            (
                "objects\\weapons\\rifle\\assault_rifle\\assault_rifle",
                "weapon",
                b"weap",
            ),
            (
                "objects\\weapons\\pistol\\plasma_pistol\\plasma_pistol",
                "weapon",
                b"weap",
            ),
            ("objects\\vehicles\\warthog\\warthog", "vehicle", b"vehi"),
            ("objects\\vehicles\\phantom\\phantom", "vehicle", b"vehi"),
        ],
    ),
    (
        "BLAM_TEST_ODSTEK",
        GameId::Halo3Odst,
        &[
            ("objects\\characters\\odst\\odst", "biped", b"bipd"),
            ("objects\\characters\\odst_recon\\odst_recon", "biped", b"bipd"),
            ("objects\\weapons\\rifle\\smg_silenced\\smg_silenced", "weapon", b"weap"),
            ("objects\\vehicles\\phantom\\phantom", "vehicle", b"vehi"),
            ("objects\\vehicles\\brute_chopper\\brute_chopper", "vehicle", b"vehi"),
        ],
    ),
    (
        "BLAM_TEST_HREK",
        GameId::HaloReach,
        &[
            ("objects\\characters\\spartans\\spartans", "biped", b"bipd"),
            ("objects\\characters\\elite\\elite", "biped", b"bipd"),
            (
                "objects\\weapons\\rifle\\assault_rifle\\assault_rifle",
                "weapon",
                b"weap",
            ),
            (
                "objects\\weapons\\melee\\energy_sword\\energy_sword",
                "weapon",
                b"weap",
            ),
            (
                "objects\\vehicles\\human\\warthog\\warthog",
                "vehicle",
                b"vehi",
            ),
            (
                "objects\\vehicles\\covenant\\banshee\\banshee",
                "vehicle",
                b"vehi",
            ),
        ],
    ),
    (
        "BLAM_TEST_H4EK",
        GameId::Halo4,
        &[
            (
                "objects\\characters\\storm_masterchief\\storm_masterchief",
                "biped",
                b"bipd",
            ),
            (
                "objects\\characters\\storm_cortana\\storm_cortana",
                "biped",
                b"bipd",
            ),
            (
                "objects\\weapons\\rifle\\storm_assault_rifle\\storm_assault_rifle",
                "weapon",
                b"weap",
            ),
            (
                "objects\\vehicles\\human\\storm_warthog\\storm_warthog",
                "vehicle",
                b"vehi",
            ),
            (
                "objects\\vehicles\\covenant\\storm_banshee\\storm_banshee",
                "vehicle",
                b"vehi",
            ),
        ],
    ),
];

/// A line per field of the preview, and for each game's group of fields a line
/// per field in it, named `group.field`, as though the group's fields were the
/// preview's own (as they were before they were grouped).
fn preview_lines(out: &mut String, preview: &super::super::RenderModelPreview) {
    let groups: [(&str, &dyn std::fmt::Debug); 6] = [
        ("preview.h2", &preview.h2),
        ("preview.gen3", &preview.gen3),
        ("preview.h3", &preview.h3),
        ("preview.reach", &preview.reach),
        ("preview.h4", &preview.h4),
        ("preview.ce", &preview.ce),
    ];
    let mut top = String::new();
    field_lines(&mut top, "preview", preview);
    let count = top.lines().count();
    for (at, row) in top.lines().enumerate() {
        let name = row.split(' ').next().unwrap_or_default();
        match groups.iter().find(|(group, _)| *group == name) {
            Some((group, value)) => inline_field_lines(out, group, *value, at + 1 == count),
            None => {
                out.push_str(row);
                out.push('\n');
            }
        }
    }
}

/// What a load produced, without the ids that count loads.
fn load_snapshot(data: &ModelPreviewData) -> String {
    let mut out = String::new();
    line(&mut out, "render_model_path", &data.render_model_path);
    preview_lines(&mut out, &data.preview);
    for variant in &data.variants {
        let mut regions: Vec<_> = variant.regions.iter().collect();
        regions.sort();
        let mut hidden: Vec<_> = variant.hidden_regions.iter().collect();
        hidden.sort();
        let text = format!("{regions:?} {hidden:?} {:?}", variant.muted_nodes);
        line(&mut out, &format!("variant {}", variant.name), &text);
    }
    line(
        &mut out,
        "default_variant",
        &format!("{:?}", data.default_variant),
    );
    line(
        &mut out,
        "change_colors",
        &format!("{:?}", data.change_colors),
    );
    line(
        &mut out,
        "ce_object_functions",
        &format!("{:?}", data.ce_object_functions),
    );
    line(&mut out, "ce_attached", &format!("{:?}", data.ce_attached));
    line(&mut out, "ce_weapons", &format!("{:?}", data.ce_weapons));
    line(
        &mut out,
        "object_functions",
        &format!("{:?}", data.object_functions),
    );
    line(&mut out, "weapons", &format!("{:?}", data.weapons));
    line(&mut out, "attached", &format!("{:?}", data.attached));
    line(
        &mut out,
        "scenario_bsps",
        &format!("{:?}", data.scenario_bsps),
    );
    line(&mut out, "notes", &format!("{:?}", data.notes));
    out
}

/// G-load: each object of [`LOAD_SET`] loads to what it did before.
#[test]
fn gate_load() {
    let Some(dir) = gate_dir() else {
        eprintln!("skipping: BABOON_GATE_DIR is not set");
        return;
    };
    let mut failures = Vec::new();
    for (variable, game, objects) in LOAD_SET {
        let Ok(root) = std::env::var(variable) else {
            eprintln!("skipping {game:?}: {variable} is not set");
            continue;
        };
        let mut snapshot = String::new();
        for (path, extension, group) in *objects {
            let started = std::time::Instant::now();
            let data = kit_preview(&root, *game, path, extension, group);
            let loaded = started.elapsed();
            // Each line names its object, so a report says whose field moved.
            let object = path.rsplit('\\').next().unwrap_or(path);
            for row in load_snapshot(&data).lines() {
                let _ = writeln!(snapshot, "{object}.{extension} {row}");
            }
            eprintln!(
                "{path}: load {loaded:.1?}, snapshot {:.1?}",
                started.elapsed() - loaded
            );
        }
        if let Err(report) =
            bless_or_compare(&dir.join("load"), &format!("{game:?}.txt"), &snapshot)
        {
            failures.push(report);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
