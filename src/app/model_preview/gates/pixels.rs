//! G-px, G-sim, G-gl and G-calls: what each object of [`PIXEL_SET`] draws,
//! keeps and leaves, in every look.

use super::*;

/// What G-px shoots of each game: a biped, a vehicle, and something that
/// moves at rest (the phantom does for Halo 3 and ODST), each in every
/// [`LOOKS`]. Reach and Halo 4 each have a Warthog for their screen lights,
/// Halo 2 a cigar for its particles, a flag for its cloth and a Banshee for
/// its contrail, Halo 4 a plasma turret and a power core for its effects'
/// volumes and tracers, and Halo CE a flag for its flag widget.
#[cfg(target_os = "macos")]
const PIXEL_SET: &[(&str, GameId, &[(&str, &str)])] = &[
    (
        "BLAM_TEST_HCEEK",
        GameId::HaloCe,
        &[
            ("characters\\cyborg\\cyborg", "biped"),
            ("vehicles\\warthog\\warthog", "vehicle"),
            ("weapons\\plasma rifle\\plasma rifle", "weapon"),
            // Its flag: still air hangs it out of sight, so the simulation gate
            // is what sees it.
            ("weapons\\flag\\flag", "weapon"),
        ],
    ),
    (
        "BLAM_TEST_H2EK",
        GameId::Halo2,
        &[
            ("objects\\characters\\masterchief\\masterchief", "biped"),
            ("objects\\vehicles\\warthog\\warthog", "vehicle"),
            (
                "objects\\weapons\\rifle\\plasma_rifle\\plasma_rifle",
                "weapon",
            ),
            // Its constant smoke: the only Halo 2 particles drawn at rest.
            ("objects\\cinematics\\cigar\\cigar", "scenery"),
            // Its cloth: the only Halo 2 cloth the set has.
            ("objects\\weapons\\multiplayer\\flag\\flag", "weapon"),
            // Its contrail and flares, drawn in the `powered` look.
            ("objects\\vehicles\\banshee\\banshee", "vehicle"),
        ],
    ),
    (
        "BLAM_TEST_H3EK",
        GameId::Halo3,
        &[
            ("objects\\characters\\masterchief\\masterchief", "biped"),
            ("objects\\vehicles\\phantom\\phantom", "vehicle"),
        ],
    ),
    (
        "BLAM_TEST_ODSTEK",
        GameId::Halo3Odst,
        &[
            ("objects\\characters\\odst\\odst", "biped"),
            ("objects\\vehicles\\brute_chopper\\brute_chopper", "vehicle"),
            ("objects\\vehicles\\phantom\\phantom", "vehicle"),
        ],
    ),
    (
        "BLAM_TEST_HREK",
        GameId::HaloReach,
        &[
            ("objects\\characters\\spartans\\spartans", "biped"),
            ("objects\\vehicles\\covenant\\banshee\\banshee", "vehicle"),
            ("objects\\characters\\cortana\\cortana", "biped"),
            // Its lights are screen-space, drawn in the `lit` look.
            ("objects\\vehicles\\human\\warthog\\warthog", "vehicle"),
        ],
    ),
    (
        "BLAM_TEST_H4EK",
        GameId::Halo4,
        &[
            (
                "objects\\characters\\storm_masterchief\\storm_masterchief",
                "biped",
            ),
            (
                "objects\\vehicles\\human\\storm_warthog\\storm_warthog",
                "vehicle",
            ),
            ("objects\\characters\\storm_cortana\\storm_cortana", "biped"),
            // Its light volume, through its material.
            (
                "objects\\vehicles\\covenant\\turrets\\plasma_turret\\plasma_turret_mounted",
                "vehicle",
            ),
            // Its tracers, from core to arms.
            (
                "environments\\shared\\crates\\cov\\cov_power_core\\cov_power_core",
                "crate",
            ),
        ],
    ),
];

/// Each way G-px shoots an object, from a plain shot in the dark theme at rest.
#[cfg(target_os = "macos")]
type Look = fn(&mut super::super::harness::shot::ShotOptions);

#[cfg(target_os = "macos")]
const LOOKS: &[(&str, Look)] = &[
    ("plain", |_| {}),
    ("light_grid", |options| {
        options.light_theme = true;
        options.grid = true;
    }),
    ("close", |options| options.scale = Some(0.35)),
    ("t0.5", |options| options.time = 0.5),
    ("t3", |options| options.time = 3.0),
    ("t10", |options| options.time = 10.0),
    // Over the grid, so the camo has something behind it to bend.
    ("camo", |options| {
        options.camo = true;
        options.grid = true;
    }),
    ("bloom", |options| options.bloom = true),
    ("shadow", |options| options.shadow = true),
    // Lights hung on these are off at rest: the Warthogs' screen lights among
    // them, which no other look draws.
    ("lit", |options| {
        options.functions = ["driver_seat_power", "brakelights", "illumination"]
            .map(|name| (name.to_owned(), 1.0))
            .to_vec();
    }),
    // Halo 2's light volumes, gels, flares and contrails are off at rest:
    // the Chief's flashlight, the Warthog's headlights and the Banshee's boost
    // turn them on, and the contrail has had time to trail.
    ("powered", |options| {
        options.time = 3.0;
        options.functions = [
            "flashlight",
            "integrated_light_power",
            "headlights",
            "flashlights_allowed",
            "driver_seat_power",
            "driver_seat_occupied",
            "speed_blended_boost",
            "boost",
        ]
        .map(|name| (name.to_owned(), 1.0))
        .to_vec();
    }),
];

/// Saves a shot as a PNG, upright.
#[cfg(target_os = "macos")]
fn save_png(path: &std::path::Path, pixels: &[u8]) {
    use super::super::harness::shot::SHOT_SIZE;
    let row = SHOT_SIZE[0] as usize * 4;
    let upright: Vec<u8> = pixels.chunks_exact(row).rev().flatten().copied().collect();
    let image =
        image::RgbaImage::from_raw(SHOT_SIZE[0], SHOT_SIZE[1], upright).expect("a whole shot");
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    image
        .save(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

/// G-px, G-sim, G-gl and G-calls: each object of [`PIXEL_SET`], in each of
/// [`LOOKS`], draws the same pixels as before, its slot keeps the same
/// simulations (a line per field, under `sim/`), the paint leaves GL as it did
/// (under `gl/`), and it makes the same GL state calls, uniforms and draws in
/// the same order (a count and hash per shot, under `glcalls/`). Blessing
/// keeps every shot as a PNG, and every call log, beside the hashes; a shot
/// that differs is saved under `changed/` to set against it.
#[cfg(target_os = "macos")]
#[test]
fn gate_pixels() {
    use super::super::harness::shot::{Shot, ShotOptions, shoot};
    use crate::core::source::TagSource;
    let Some(dir) = gate_dir() else {
        eprintln!("skipping: BABOON_GATE_DIR is not set");
        return;
    };
    let (sim_dir, gl_dir, calls_dir, dir) = (dir.join("sim"), dir.join("gl"), dir.join("glcalls"), dir.join("px"));
    let bless = std::env::var_os("BABOON_GATE_BLESS").is_some();
    let mut failures = Vec::new();
    for (variable, game, objects) in PIXEL_SET {
        let Ok(root) = std::env::var(variable) else {
            eprintln!("skipping {game:?}: {variable} is not set");
            continue;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(*game),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let mut names = Vec::new();
        let mut shots = Vec::new();
        for (path, kind) in *objects {
            for (look, adjust) in LOOKS {
                let shot = Shot {
                    model: (*path).to_owned(),
                    shaders: true,
                    lights: true,
                    yaw: 0.6,
                    pitch: 0.25,
                    kind: Some((*kind).to_owned()),
                };
                let mut options = ShotOptions::new(*game);
                adjust(&mut options);
                let object = path.rsplit('\\').next().unwrap_or(path);
                names.push(format!("{object}_{kind}_{look}"));
                shots.push((shot, options));
            }
        }
        let started = std::time::Instant::now();
        let images = shoot(&source, &shots);
        eprintln!(
            "{game:?}: {} shots in {:.1?}",
            images.len(),
            started.elapsed()
        );
        // No draw may sample what it draws into: GL leaves that undefined,
        // and its result can follow the GPU's timing.
        let mut loops = std::collections::BTreeMap::new();
        for (name, shown) in names.iter().zip(&images) {
            for line in &shown.feedback_loops {
                loops.entry(line.clone()).or_insert_with(Vec::new).push(name.clone());
            }
        }
        if !loops.is_empty() {
            let mut report = format!("{game:?} draws sample what they draw into:\n");
            for (line, shots) in &loops {
                let _ = writeln!(report, "  {line} ({} draws, first in {})", shots.len(), shots[0]);
            }
            failures.push(report);
        }
        let mut snapshot = String::new();
        let mut simulations = String::new();
        let mut gl_states = String::new();
        let mut calls = String::new();
        for (name, shown) in names.iter().zip(&images) {
            let log = shown.gl_calls.join("\n");
            let hash = log.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            });
            let _ = writeln!(calls, "{name} {} {hash:016x}", shown.gl_calls.len());
            for row in shown.simulation.lines() {
                let _ = writeln!(simulations, "{name} {row}");
            }
            for row in shown.gl_state.lines() {
                let _ = writeln!(gl_states, "{name} {row}");
            }
            let pixels = &shown.pixels;
            let hash = pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
            });
            let _ = writeln!(snapshot, "{name} {hash:016x}");
            if bless {
                save_png(
                    &dir.join(format!("{game:?}")).join(format!("{name}.png")),
                    pixels,
                );
            }
        }
        let simulated = bless_or_compare(&sim_dir, &format!("{game:?}.txt"), &simulations);
        failures.extend(simulated.err());
        let left = bless_or_compare(&gl_dir, &format!("{game:?}.txt"), &gl_states);
        failures.extend(left.err());
        // G-calls: every GL state call, uniform and draw each shot made, in
        // order. A shot that differs keeps its whole log under `changed/`, to
        // set against the one blessing keeps beside the hashes.
        let save_log = |folder: &std::path::Path, name: &str, shown: &super::super::harness::shot::Shown| {
            let path = folder.join(format!("{game:?}")).join(format!("{name}.log"));
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, shown.gl_calls.join("\n"));
        };
        if bless {
            for (name, shown) in names.iter().zip(&images) {
                save_log(&calls_dir, name, shown);
            }
        }
        if let Err(report) = bless_or_compare(&calls_dir, &format!("{game:?}.txt"), &calls) {
            for (name, shown) in names.iter().zip(&images) {
                if report.contains(&format!("+ {name} ")) {
                    save_log(&calls_dir.join("changed"), name, shown);
                }
            }
            failures.push(report);
        }
        if let Err(report) = bless_or_compare(&dir, &format!("{game:?}.txt"), &snapshot) {
            for (name, shown) in names.iter().zip(&images) {
                let pixels = &shown.pixels;
                if report.contains(&format!("+ {name} ")) {
                    save_png(
                        &dir.join("changed")
                            .join(format!("{game:?}"))
                            .join(format!("{name}.png")),
                        pixels,
                    );
                }
            }
            failures.push(report);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
