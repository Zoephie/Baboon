//! Refactor gates: snapshots of what the preview produces, compared before and
//! after a change that should not alter it.
//!
//! `BABOON_GATE_DIR` names a folder outside the repo for the baselines;
//! `BABOON_GATE_BLESS=1` writes them instead of comparing. Each game's objects
//! load from its kit, named by the usual `BLAM_TEST_*` variable; a kit that is
//! not set is skipped and says so.

use std::fmt::Write as _;
use std::path::PathBuf;

use super::load::kit_object_tests::kit_preview;
use super::state::ModelPreviewData;
use crate::core::game::GameId;

/// The objects each game loads: a biped, weapons and vehicles, chosen to reach
/// as many of the preview's features as one load can.
pub(super) const LOAD_SET: &[(&str, GameId, &[(&str, &str, &[u8; 4])])] = &[
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

/// FNV-1a, so a baseline means the same under any toolchain.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// One line for a value: its name, the length of its `Debug` text and that
/// text's hash.
fn line(out: &mut String, name: &str, text: &str) {
    let _ = writeln!(out, "{name} {} {:016x}", text.len(), fnv1a(text));
}

/// Hashes a struct's compact `Debug` as it is written, a line per top-level
/// field, so a difference names the field it is in without the whole text
/// ever being held (a Halo 4 object's particles run to gigabytes). Fields are
/// found by bracket depth, outside quoted text, and only in a braced struct:
/// anything else (a tuple struct, a map) is one line under the prefix.
struct FieldLines<'a> {
    out: &'a mut String,
    prefix: &'a str,
    /// The field being hashed: its name, length and hash so far.
    field: Option<(String, usize, u64)>,
    /// Brackets open, counting the struct's own.
    depth: usize,
    quoted: bool,
    escaped: bool,
    /// At the struct's own level, a field's name as it is read, up to `: `.
    naming: Option<String>,
    /// The value opened with `{` after a type's name, so is a struct with
    /// named fields (a bare map opens with `{` too).
    braced: bool,
    /// What came before the value's first bracket.
    named: String,
    /// Every byte, for a value that is not a braced struct.
    whole: (usize, u64),
    /// Whether any field's line was written.
    split: bool,
}

impl FieldLines<'_> {
    fn finish_field(&mut self) {
        if let Some((name, length, hash)) = self.field.take() {
            let _ = writeln!(self.out, "{}.{name} {length} {hash:016x}", self.prefix);
            self.split = true;
        }
    }

    /// The last field's line, or the whole value's if it had none.
    fn close(mut self) {
        self.finish_field();
        if !self.split {
            let (length, hash) = self.whole;
            let _ = writeln!(self.out, "{} {length} {hash:016x}", self.prefix);
        }
    }

    fn step(&mut self, byte: u8) {
        let (length, hash) = &mut self.whole;
        *length += 1;
        *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        if let Some(name) = &mut self.naming {
            if byte == b' ' && name.ends_with(':') {
                name.pop();
                let name = self.naming.take().unwrap_or_default().trim().to_owned();
                self.finish_field();
                self.field = Some((name, 0, 0xcbf2_9ce4_8422_2325));
            } else {
                name.push(char::from(byte));
            }
            return;
        }
        if let Some((_, length, hash)) = &mut self.field {
            *length += 1;
            *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        if self.quoted {
            match byte {
                _ if self.escaped => self.escaped = false,
                b'\\' => self.escaped = true,
                b'"' => self.quoted = false,
                _ => {}
            }
            return;
        }
        if self.depth == 0 && self.named.len() < 128 && !matches!(byte, b'{' | b'[' | b'(') {
            self.named.push(char::from(byte));
        }
        match byte {
            b'"' => self.quoted = true,
            b'{' | b'[' | b'(' => {
                self.depth += 1;
                let name = self.named.trim();
                let struct_name =
                    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
                if self.depth == 1 && byte == b'{' && struct_name {
                    self.braced = true;
                    self.naming = Some(String::new());
                }
            }
            b'}' | b']' | b')' => self.depth = self.depth.saturating_sub(1),
            b',' if self.depth == 1 && self.braced => self.naming = Some(String::new()),
            _ => {}
        }
    }
}

impl std::fmt::Write for FieldLines<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        text.bytes().for_each(|byte| self.step(byte));
        Ok(())
    }
}

impl<'a> FieldLines<'a> {
    fn new(out: &'a mut String, prefix: &'a str) -> Self {
        FieldLines {
            out,
            prefix,
            field: None,
            depth: 0,
            quoted: false,
            escaped: false,
            naming: None,
            braced: false,
            named: String::new(),
            whole: (0, 0xcbf2_9ce4_8422_2325),
            split: false,
        }
    }
}

/// A line per top-level field of `value`'s `Debug`.
pub(super) fn field_lines(out: &mut String, prefix: &str, value: &impl std::fmt::Debug) {
    let mut lines = FieldLines::new(out, prefix);
    let _ = write!(lines, "{value:?}");
    lines.close();
}

/// A line per field of a struct that is itself a field, read as though its
/// fields were its holder's own: its closing ` }` becomes the `,` that would
/// follow them there, or stays for the holder's `last` field.
fn inline_field_lines(out: &mut String, prefix: &str, value: &dyn std::fmt::Debug, last: bool) {
    /// The bytes but the last two, which are the struct's ` }`.
    struct Held<'a> {
        lines: FieldLines<'a>,
        held: std::collections::VecDeque<u8>,
    }
    impl std::fmt::Write for Held<'_> {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            for byte in text.bytes() {
                self.held.push_back(byte);
                if self.held.len() > 2 {
                    let byte = self.held.pop_front().unwrap_or_default();
                    self.lines.step(byte);
                }
            }
            Ok(())
        }
    }
    let mut held = Held { lines: FieldLines::new(out, prefix), held: std::collections::VecDeque::new() };
    let _ = write!(held, "{value:?}");
    let Held { mut lines, held } = held;
    let ending: Vec<u8> = if held.iter().eq(b" }") && !last { b",".to_vec() } else { held.into() };
    ending.into_iter().for_each(|byte| lines.step(byte));
    lines.close();
}

/// A line per field of the preview, and for each game's group of fields a line
/// per field in it, named `group.field`, as though the group's fields were the
/// preview's own (as they were before they were grouped).
fn preview_lines(out: &mut String, preview: &super::RenderModelPreview) {
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

fn gate_dir() -> Option<PathBuf> {
    std::env::var_os("BABOON_GATE_DIR").map(PathBuf::from)
}

/// Writes `snapshot` as `name`'s baseline, or compares it with the one there,
/// naming each line that differs.
fn bless_or_compare(dir: &std::path::Path, name: &str, snapshot: &str) -> Result<(), String> {
    let path = dir.join(name);
    if std::env::var_os("BABOON_GATE_BLESS").is_some() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        std::fs::write(&path, snapshot).map_err(|error| error.to_string())?;
        eprintln!("blessed {}", path.display());
        return Ok(());
    }
    let baseline =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if baseline == snapshot {
        return Ok(());
    }
    let (old, new): (Vec<_>, Vec<_>) = (baseline.lines().collect(), snapshot.lines().collect());
    let mut report = format!("{name} differs from its baseline:\n");
    for gone in old.iter().filter(|row| !new.contains(row)) {
        let _ = writeln!(report, "  - {gone}");
    }
    for came in new.iter().filter(|row| !old.contains(row)) {
        let _ = writeln!(report, "  + {came}");
    }
    if old.iter().all(|row| new.contains(row)) && new.iter().all(|row| old.contains(row)) {
        report.push_str("  (the same lines, in another order)\n");
    }
    Err(report)
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
type Look = fn(&mut super::renderer::headless_shot::ShotOptions);

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
    use super::renderer::headless_shot::SHOT_SIZE;
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
/// (under `gl/`), and, with `BABOON_GATE_CALLS` set, it makes the same GL
/// state calls, uniforms and draws in the same order (a count and hash per
/// shot, under `glcalls/`). Blessing
/// keeps every shot as a PNG, and every call log, beside the hashes; a shot
/// that differs is saved under `changed/` to set against it.
#[cfg(target_os = "macos")]
#[test]
fn gate_pixels() {
    use super::renderer::headless_shot::{Shot, ShotOptions, shoot};
    use crate::core::source::TagSource;
    let Some(dir) = gate_dir() else {
        eprintln!("skipping: BABOON_GATE_DIR is not set");
        return;
    };
    let (sim_dir, gl_dir, calls_dir, dir) = (dir.join("sim"), dir.join("gl"), dir.join("glcalls"), dir.join("px"));
    let bless = std::env::var_os("BABOON_GATE_BLESS").is_some();
    let calls_gated = std::env::var_os("BABOON_GATE_CALLS").is_some();
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
        // G-calls, with `BABOON_GATE_CALLS` set: every GL state call, uniform
        // and draw each shot made, in order. A shot that differs keeps its
        // whole log under `changed/`, to set against the one blessing keeps
        // beside the hashes. Opt-in: Halo 4's colour grading is cached by its
        // image's address, so whether a shot re-uploads it (one
        // `glBindTexture` of a 3D map) varies from run to run.
        let save_log = |folder: &std::path::Path, name: &str, shown: &super::renderer::headless_shot::Shown| {
            let path = folder.join(format!("{game:?}")).join(format!("{name}.log"));
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, shown.gl_calls.join("\n"));
        };
        if calls_gated {
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

/// What the code read an option as before reading it by name.
#[derive(Clone, Copy, Debug)]
enum Old {
    /// `value & bit != 0`.
    Bit(i128),
    /// `value == n`.
    Value(i128),
    /// A set option whose stored name starts with one of these, as
    /// `read_flag_names` and `starts_with` read it; an option the layout
    /// lacks reads unset by name.
    Prefix(&'static [&'static str]),
}

/// G-opt: one option read the old way and by name, over every kit tag of a
/// group. `path` walks blocks to the field: each segment but the last is a
/// block whose every element is read.
struct OptionRow {
    kit: &'static str,
    game: GameId,
    extension: &'static str,
    path: &'static str,
    old: Old,
    name: &'static str,
    /// A control, which must disagree, so the gate shows it can fail.
    control: bool,
    /// The option is the one after `name`, for one whose own name another
    /// option shares (`... forward/backward` after each kind of loop).
    after: bool,
    /// What older layouts label the option, tried after `name`.
    older: &'static [&'static str],
}

impl OptionRow {
    /// `name`, then its older labels.
    fn labels(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.older.iter().copied())
    }
}

const fn row(kit: &'static str, game: GameId, extension: &'static str, path: &'static str, old: Old, name: &'static str) -> OptionRow {
    OptionRow { kit, game, extension, path, old, name, control: false, after: false, older: &[] }
}

/// A row whose option older layouts label otherwise.
const fn labelled(
    kit: &'static str,
    game: GameId,
    extension: &'static str,
    path: &'static str,
    old: Old,
    name: &'static str,
    older: &'static [&'static str],
) -> OptionRow {
    OptionRow { older, ..row(kit, game, extension, path, old, name) }
}

const fn after(kit: &'static str, game: GameId, extension: &'static str, path: &'static str, old: Old, name: &'static str) -> OptionRow {
    OptionRow { after: true, ..row(kit, game, extension, path, old, name) }
}

const HCE: (&str, GameId) = ("BLAM_TEST_HCEEK", GameId::HaloCe);
const H2: (&str, GameId) = ("BLAM_TEST_H2EK", GameId::Halo2);
const H3: (&str, GameId) = ("BLAM_TEST_H3EK", GameId::Halo3);
const ODST: (&str, GameId) = ("BLAM_TEST_ODSTEK", GameId::Halo3Odst);
const REACH: (&str, GameId) = ("BLAM_TEST_HREK", GameId::HaloReach);
const H4: (&str, GameId) = ("BLAM_TEST_H4EK", GameId::Halo4);

/// Every option the refactor reads by name, against the number it replaced.
const OPTION_ROWS: &[OptionRow] = &[
    // A control: the wrong name for a bit.
    OptionRow { control: true, ..row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x1), "dies at rest") },
    // ce::particles
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x1), "can animate backwards"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x2), "animation stops at rest"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x4), "animation starts on random frame"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x8), "animate once per frame"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x10), "dies at rest"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x20), "dies on contact with structure"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x40), "tint from diffuse texture"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x100), "dies on contact with air"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x200), "self-illuminated"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x400), "random horizontal mirroring"),
    row(HCE.0, HCE.1, "particle", "flags", Old::Bit(0x800), "random vertical mirroring"),
    row(HCE.0, HCE.1, "particle", "shader flags", Old::Bit(0x2), "nonlinear tint"),
    row(HCE.0, HCE.1, "point_physics", "flags", Old::Bit(0x2), "collides with structures"),
    row(HCE.0, HCE.1, "point_physics", "flags", Old::Bit(0x20), "no gravity"),
    row(H2.0, H2.1, "point_physics", "flags", Old::Bit(0x2), "collides with structures"),
    row(H2.0, H2.1, "point_physics", "flags", Old::Bit(0x20), "no gravity"),
    row(HCE.0, HCE.1, "effect", "events/parts/flags", Old::Bit(0x1), "face down regardless of location (decals)"),
    row(HCE.0, HCE.1, "effect", "events/particles/flags", Old::Bit(0x8), "interpolate tint as HSV"),
    row(HCE.0, HCE.1, "effect", "events/particles/flags", Old::Bit(0x10), "... across the long hue path"),
    row(HCE.0, HCE.1, "effect", "events/particles/distribution function", Old::Value(0), "start"),
    row(HCE.0, HCE.1, "effect", "events/particles/distribution function", Old::Value(1), "end"),
    row(HCE.0, HCE.1, "effect", "events/particles/distribution function", Old::Value(2), "constant"),
    row(HCE.0, HCE.1, "effect", "events/particles/distribution function", Old::Value(3), "buildup"),
    row(HCE.0, HCE.1, "effect", "events/particles/distribution function", Old::Value(4), "falloff"),
    row(HCE.0, HCE.1, "effect", "events/particles/distribution function", Old::Value(5), "buildup and falloff"),
    row(HCE.0, HCE.1, "particle", "anchor", Old::Value(0), "with primary"),
    row(HCE.0, HCE.1, "particle", "anchor", Old::Value(1), "with screen space"),
    row(HCE.0, HCE.1, "particle", "anchor", Old::Value(2), "zsprite"),
    // ce::contrails
    row(HCE.0, HCE.1, "contrail", "flags", Old::Bit(0x1), "first point unfaded"),
    row(HCE.0, HCE.1, "contrail", "flags", Old::Bit(0x2), "last point unfaded"),
    row(HCE.0, HCE.1, "contrail", "flags", Old::Bit(0x40), "edge effect fades slowly"),
    row(HCE.0, HCE.1, "contrail", "shader flags", Old::Bit(0x2), "nonlinear tint"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x1), "point generation rate"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x2), "point velocity"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x4), "point velocity delta"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x8), "point velocity cone angle"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x20), "sequence animation rate"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x40), "texture scale u"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x80), "texture scale v"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x100), "texture animation u"),
    row(HCE.0, HCE.1, "contrail", "scale flags", Old::Bit(0x200), "texture animation v"),
    row(HCE.0, HCE.1, "contrail", "point states/scale flags", Old::Bit(0x1), "duration"),
    row(HCE.0, HCE.1, "contrail", "point states/scale flags", Old::Bit(0x2), "duration delta"),
    row(HCE.0, HCE.1, "contrail", "point states/scale flags", Old::Bit(0x4), "transition duration"),
    row(HCE.0, HCE.1, "contrail", "point states/scale flags", Old::Bit(0x8), "transition duration delta"),
    row(HCE.0, HCE.1, "contrail", "point states/scale flags", Old::Bit(0x10), "width"),
    row(HCE.0, HCE.1, "contrail", "point states/scale flags", Old::Bit(0x20), "color"),
    row(HCE.0, HCE.1, "contrail", "render type", Old::Value(0), "vertical orientation"),
    row(HCE.0, HCE.1, "contrail", "render type", Old::Value(1), "horizontal orientation"),
    row(HCE.0, HCE.1, "contrail", "render type", Old::Value(2), "media mapped"),
    row(HCE.0, HCE.1, "contrail", "render type", Old::Value(3), "ground mapped"),
    row(HCE.0, HCE.1, "contrail", "render type", Old::Value(4), "viewer facing"),
    row(HCE.0, HCE.1, "contrail", "render type", Old::Value(5), "double-marker linked"),
    row(HCE.0, HCE.1, "contrail", "framebuffer fade mode", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "contrail", "framebuffer fade mode", Old::Value(1), "fade when perpendicular"),
    row(HCE.0, HCE.1, "contrail", "framebuffer fade mode", Old::Value(2), "fade when parallel"),
    // ce::widgets
    row(HCE.0, HCE.1, "light_volume", "flags", Old::Bit(0x1), "interpolate color in hsv"),
    row(HCE.0, HCE.1, "light_volume", "flags", Old::Bit(0x2), "...more colors"),
    row(HCE.0, HCE.1, "lightning", "markers/flags", Old::Bit(0x1), "not connected to next marker"),
    row(HCE.0, HCE.1, "lightning", "shader/shader flags", Old::Bit(0x2), "nonlinear tint"),
    row(HCE.0, HCE.1, "bitmap", "type", Old::Value(3), "sprites"),
    row(HCE.0, HCE.1, "glow", "glow flags", Old::Bit(0x1), "modify particle color in range"),
    row(HCE.0, HCE.1, "glow", "glow flags", Old::Bit(0x2), "particles move backwards"),
    row(HCE.0, HCE.1, "glow", "glow flags", Old::Bit(0x4), "particles move in both directions"),
    row(HCE.0, HCE.1, "glow", "glow flags", Old::Bit(0x8), "trailing particles fade over time"),
    row(HCE.0, HCE.1, "glow", "glow flags", Old::Bit(0x10), "trailing particles shrink over time"),
    row(HCE.0, HCE.1, "glow", "glow flags", Old::Bit(0x20), "trailing particles slow over time"),
    row(HCE.0, HCE.1, "glow", "boundary effect", Old::Value(0), "bounce"),
    row(HCE.0, HCE.1, "glow", "boundary effect", Old::Value(1), "wrap"),
    row(HCE.0, HCE.1, "glow", "normal particle distribution", Old::Value(0), "distributed randomly"),
    row(HCE.0, HCE.1, "glow", "normal particle distribution", Old::Value(1), "distributed uniformly"),
    row(HCE.0, HCE.1, "glow", "trailing particle distribution", Old::Value(0), "emit vertically"),
    row(HCE.0, HCE.1, "glow", "trailing particle distribution", Old::Value(1), "emit normal (up)"),
    row(HCE.0, HCE.1, "glow", "trailing particle distribution", Old::Value(2), "emit randomly"),
    // ce::passes
    row(HCE.0, HCE.1, "shader_transparent_chicago", "flags", Old::Bit(0x1), "alpha-tested"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "flags", Old::Bit(0x2), "decal"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "flags", Old::Bit(0x4), "two-sided"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "flags", Old::Bit(0x8), "first map is in screenspace"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "flags", Old::Bit(0x80), "numeric"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "maps/flags", Old::Bit(0x2), "alpha replicate"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "maps/flags", Old::Bit(0x4), "u-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "maps/flags", Old::Bit(0x8), "v-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "extra flags", Old::Bit(0x1), "don't fade active-camouflage"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "extra flags", Old::Bit(0x2), "numeric countdown timer"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "extra flags", Old::Bit(0x4), "custom edition blending"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "flags", Old::Bit(0x1), "alpha-tested"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "flags", Old::Bit(0x2), "decal"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "flags", Old::Bit(0x4), "two-sided"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "flags", Old::Bit(0x8), "first map is in screenspace"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "flags", Old::Bit(0x80), "numeric"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "4 stage maps/flags", Old::Bit(0x2), "alpha replicate"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "4 stage maps/flags", Old::Bit(0x4), "u-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "4 stage maps/flags", Old::Bit(0x8), "v-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "2 stage maps/flags", Old::Bit(0x2), "alpha replicate"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "2 stage maps/flags", Old::Bit(0x4), "u-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "2 stage maps/flags", Old::Bit(0x8), "v-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "extra flags", Old::Bit(0x1), "don't fade active-camouflage"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "extra flags", Old::Bit(0x2), "numeric countdown timer"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "extra flags", Old::Bit(0x4), "custom edition blending"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "flags", Old::Bit(0x1), "alpha-tested"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "flags", Old::Bit(0x2), "decal"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "flags", Old::Bit(0x4), "two-sided"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "flags", Old::Bit(0x8), "first map is in screenspace"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "flags", Old::Bit(0x80), "numeric"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "maps/flags", Old::Bit(0x2), "u-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "maps/flags", Old::Bit(0x4), "v-clamped"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "stages/flags", Old::Bit(0x1), "color mux"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "stages/flags", Old::Bit(0x2), "alpha mux"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "stages/flags", Old::Bit(0x4), "A-out controls color0 animation"),
    row(HCE.0, HCE.1, "shader_transparent_glass", "flags", Old::Bit(0x1), "alpha-tested"),
    row(HCE.0, HCE.1, "shader_transparent_glass", "flags", Old::Bit(0x2), "decal"),
    row(HCE.0, HCE.1, "shader_transparent_glass", "flags", Old::Bit(0x4), "two-sided"),
    row(HCE.0, HCE.1, "shader_transparent_glass", "flags", Old::Bit(0x8), "bump map is specular mask"),
    row(HCE.0, HCE.1, "shader_transparent_meter", "flags", Old::Bit(0x1), "decal"),
    row(HCE.0, HCE.1, "shader_transparent_meter", "flags", Old::Bit(0x2), "two-sided"),
    row(HCE.0, HCE.1, "shader_transparent_meter", "flags", Old::Bit(0x4), "flash color is negative"),
    row(HCE.0, HCE.1, "shader_transparent_meter", "flags", Old::Bit(0x8), "tint mode-2"),
    row(HCE.0, HCE.1, "shader_transparent_water", "flags", Old::Bit(0x1), "base map alpha modulates reflection"),
    row(HCE.0, HCE.1, "shader_transparent_water", "flags", Old::Bit(0x2), "base map color modulates background"),
    row(HCE.0, HCE.1, "shader_model", "flags", Old::Bit(0x1), "detail after reflection"),
    row(HCE.0, HCE.1, "shader_model", "flags", Old::Bit(0x2), "two-sided"),
    row(HCE.0, HCE.1, "shader_model", "flags", Old::Bit(0x4), "not alpha-tested"),
    row(HCE.0, HCE.1, "shader_model", "flags", Old::Bit(0x8), "alpha-blended decal"),
    row(HCE.0, HCE.1, "shader_model", "flags", Old::Bit(0x40), "multipurpose map uses OG Xbox channel order"),
    row(HCE.0, HCE.1, "globals", "rasterizer data/flags", Old::Bit(0x1), "tint edge density"),
    row(HCE.0, HCE.1, "shader_environment", "flags", Old::Bit(0x1), "alpha-tested"),
    // ce::lights
    row(HCE.0, HCE.1, "lens_flare", "reflections/flags", Old::Bit(0x1), "align rotation with screen center"),
    row(HCE.0, HCE.1, "lens_flare", "reflections/flags", Old::Bit(0x2), "radius NOT scaled by distance"),
    row(HCE.0, HCE.1, "lens_flare", "reflections/flags", Old::Bit(0x4), "radius scaled by occlusion factor"),
    row(HCE.0, HCE.1, "lens_flare", "reflections/flags", Old::Bit(0x1), "interpolate color in hsv"),
    row(HCE.0, HCE.1, "lens_flare", "reflections/flags", Old::Bit(0x2), "...more colors"),
    row(HCE.0, HCE.1, "lens_flare", "occlusion offset direction", Old::Value(0), "toward viewer"),
    row(HCE.0, HCE.1, "lens_flare", "occlusion offset direction", Old::Value(1), "marker forward"),
    row(HCE.0, HCE.1, "lens_flare", "occlusion offset direction", Old::Value(2), "none"),
    row(HCE.0, HCE.1, "light", "flags", Old::Bit(0x1), "dynamic"),
    row(HCE.0, HCE.1, "light", "flags", Old::Bit(0x4), "don't light own object"),
    row(HCE.0, HCE.1, "light", "interpolation flags", Old::Bit(0x1), "blend in hsv"),
    row(HCE.0, HCE.1, "light", "interpolation flags", Old::Bit(0x2), "...more colors"),
    // CE function slots (the kit's equipment, garbage and placeholders carry no attachments)
    row(HCE.0, HCE.1, "biped", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "biped", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "biped", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "biped", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "biped", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "biped", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "biped", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "biped", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "biped", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "biped", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "biped", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "biped", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "biped", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "biped", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "biped", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "vehicle", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "vehicle", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "vehicle", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "vehicle", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "vehicle", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "vehicle", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "vehicle", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "vehicle", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "weapon", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "weapon", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "weapon", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "weapon", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "weapon", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "weapon", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "weapon", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "weapon", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "weapon", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "weapon", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "weapon", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "weapon", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "weapon", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "weapon", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "weapon", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "projectile", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "projectile", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "projectile", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "projectile", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "projectile", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "projectile", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "projectile", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "projectile", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "projectile", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "projectile", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "projectile", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "projectile", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "projectile", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "projectile", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "projectile", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "scenery", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "scenery", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "scenery", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "scenery", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "scenery", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "scenery", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "scenery", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "scenery", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "scenery", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "scenery", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "scenery", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "scenery", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "scenery", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "scenery", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "scenery", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "device_machine", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_machine", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_machine", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "device_machine", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_machine", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "device_machine", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "device_machine", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "device_machine", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "device_control", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_control", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "device_control", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "device_control", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "device_control", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "device_control", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_control", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "device_control", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "device_control", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "device_control", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "device_control", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_control", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "device_control", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "device_control", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "device_control", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "device_light_fixture", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/primary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/primary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/primary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/primary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/primary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/secondary scale", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/secondary scale", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/secondary scale", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/secondary scale", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/secondary scale", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/change color", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/change color", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/change color", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/change color", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "sound_scenery", "attachments/change color", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "light_volume", "brightness scale source", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "light_volume", "brightness scale source", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "light_volume", "brightness scale source", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "light_volume", "brightness scale source", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "light_volume", "brightness scale source", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "lightning", "jitter scale source", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "lightning", "jitter scale source", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "lightning", "jitter scale source", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "lightning", "jitter scale source", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "lightning", "jitter scale source", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "lightning", "thickness scale source", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "lightning", "thickness scale source", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "lightning", "thickness scale source", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "lightning", "thickness scale source", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "lightning", "thickness scale source", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "lightning", "tint modulation source", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "lightning", "tint modulation source", Old::Value(1), "A"),
    row(HCE.0, HCE.1, "lightning", "tint modulation source", Old::Value(2), "B"),
    row(HCE.0, HCE.1, "lightning", "tint modulation source", Old::Value(3), "C"),
    row(HCE.0, HCE.1, "lightning", "tint modulation source", Old::Value(4), "D"),
    row(HCE.0, HCE.1, "lightning", "brightness scale source", Old::Value(0), "none"),
    row(HCE.0, HCE.1, "lightning", "brightness scale source", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "lightning", "brightness scale source", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "lightning", "brightness scale source", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "lightning", "brightness scale source", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "glow", "attachment#0", Old::Value(0), "NONE"),
    row(HCE.0, HCE.1, "glow", "attachment#0", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "glow", "attachment#0", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "glow", "attachment#0", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "glow", "attachment#0", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "glow", "attachment#1", Old::Value(0), "NONE"),
    row(HCE.0, HCE.1, "glow", "attachment#1", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "glow", "attachment#1", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "glow", "attachment#1", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "glow", "attachment#1", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "glow", "attachment#2", Old::Value(0), "NONE"),
    row(HCE.0, HCE.1, "glow", "attachment#2", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "glow", "attachment#2", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "glow", "attachment#2", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "glow", "attachment#2", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "glow", "attachment#3", Old::Value(0), "NONE"),
    row(HCE.0, HCE.1, "glow", "attachment#3", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "glow", "attachment#3", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "glow", "attachment#3", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "glow", "attachment#3", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "glow", "attachment#4", Old::Value(0), "NONE"),
    row(HCE.0, HCE.1, "glow", "attachment#4", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "glow", "attachment#4", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "glow", "attachment#4", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "glow", "attachment#4", Old::Value(4), "D out"),
    row(HCE.0, HCE.1, "glow", "attachment#5", Old::Value(0), "NONE"),
    row(HCE.0, HCE.1, "glow", "attachment#5", Old::Value(1), "A out"),
    row(HCE.0, HCE.1, "glow", "attachment#5", Old::Value(2), "B out"),
    row(HCE.0, HCE.1, "glow", "attachment#5", Old::Value(3), "C out"),
    row(HCE.0, HCE.1, "glow", "attachment#5", Old::Value(4), "D out"),
    // CE framebuffer blend function
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(0), "alpha blend"),
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(1), "multiply"),
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(2), "double multiply"),
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(3), "add"),
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(4), "subtract"),
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(5), "component min"),
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(6), "component max"),
    row(HCE.0, HCE.1, "particle", "framebuffer blend function", Old::Value(7), "alpha-multiply add"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(0), "alpha blend"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(1), "multiply"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(2), "double multiply"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(3), "add"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(4), "subtract"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(5), "component min"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(6), "component max"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/framebuffer blend function", Old::Value(7), "alpha-multiply add"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(0), "alpha blend"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(1), "multiply"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(2), "double multiply"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(3), "add"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(4), "subtract"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(5), "component min"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(6), "component max"),
    row(HCE.0, HCE.1, "contrail", "framebuffer blend function", Old::Value(7), "alpha-multiply add"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(0), "alpha blend"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(1), "multiply"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(2), "double multiply"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(3), "add"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(4), "subtract"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(5), "component min"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(6), "component max"),
    row(HCE.0, HCE.1, "lightning", "shader/framebuffer blend function", Old::Value(7), "alpha-multiply add"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(0), "alpha blend"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(1), "multiply"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(2), "double multiply"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(3), "add"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(4), "subtract"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(5), "component min"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(6), "component max"),
    row(HCE.0, HCE.1, "shader_transparent_chicago", "framebuffer blend function", Old::Value(7), "alpha-multiply add"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(0), "alpha blend"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(1), "multiply"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(2), "double multiply"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(3), "add"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(4), "subtract"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(5), "component min"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(6), "component max"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "framebuffer blend function", Old::Value(7), "alpha-multiply add"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(0), "alpha blend"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(1), "multiply"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(2), "double multiply"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(3), "add"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(4), "subtract"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(5), "component min"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(6), "component max"),
    row(HCE.0, HCE.1, "shader_transparent_generic", "framebuffer blend function", Old::Value(7), "alpha-multiply add"),
    // h2::lights
    row(H2.0, H2.1, "lens_flare", "flags", Old::Bit(0x2), "no occlusion test"),
    row(H2.0, H2.1, "lens_flare", "flags", Old::Bit(0x4), "only render in first person"),
    row(H2.0, H2.1, "lens_flare", "flags", Old::Bit(0x40), "scale by marker"),
    row(H2.0, H2.1, "lens_flare", "flags#1", Old::Bit(0x1), "synchronized"),
    row(H2.0, H2.1, "lens_flare", "reflections/flags", Old::Bit(0x1), "align rotation with screen center"),
    row(H2.0, H2.1, "lens_flare", "reflections/flags", Old::Bit(0x2), "radius NOT scaled by distance"),
    row(H2.0, H2.1, "lens_flare", "reflections/flags", Old::Bit(0x4), "radius scaled by occlusion factor"),
    row(H2.0, H2.1, "lens_flare", "reflections/flags", Old::Bit(0x8), "occluded by solid objects"),
    row(H2.0, H2.1, "lens_flare", "reflections/flags", Old::Bit(0x10), "ignore light color"),
    row(H2.0, H2.1, "lens_flare", "reflections/flags", Old::Bit(0x20), "not affected by inner occlusion"),
    row(H2.0, H2.1, "lens_flare", "occlusion offset direction", Old::Value(0), "toward viewer"),
    row(H2.0, H2.1, "lens_flare", "occlusion offset direction", Old::Value(1), "marker forward"),
    row(H2.0, H2.1, "lens_flare", "occlusion offset direction", Old::Value(2), "none"),
    row(H2.0, H2.1, "light", "flags", Old::Bit(0x1), "no illumination"),
    row(H2.0, H2.1, "light", "flags", Old::Bit(0x2), "no specular"),
    row(H2.0, H2.1, "light", "flags", Old::Bit(0x20), "only render in first person"),
    row(H2.0, H2.1, "light", "flags", Old::Bit(0x800), "ignore parent object"),
    row(H2.0, H2.1, "light", "flags", Old::Bit(0x2000), "ignore all parents"),
    row(H2.0, H2.1, "light", "type", Old::Value(0), "sphere"),
    row(H2.0, H2.1, "light", "type", Old::Value(1), "orthogonal"),
    row(H2.0, H2.1, "light", "type", Old::Value(2), "projective"),
    row(H2.0, H2.1, "light", "type", Old::Value(3), "pyramid"),
    row(H2.0, H2.1, "light", "falloff geometry", Old::Value(0), "default"),
    row(H2.0, H2.1, "light", "falloff geometry", Old::Value(1), "directional"),
    row(H2.0, H2.1, "light", "falloff geometry", Old::Value(2), "spherical"),
    row(H2.0, H2.1, "light", "default lightmap setting", Old::Value(0), "dynamic only"),
    row(H2.0, H2.1, "light", "default lightmap setting", Old::Value(1), "dynamic with lightmaps"),
    row(H2.0, H2.1, "light", "default lightmap setting", Old::Value(2), "lightmaps only"),
    row(H2.0, H2.1, "light", "diffuse contrast", Old::Value(0), "default (linear)"),
    row(H2.0, H2.1, "light", "diffuse contrast", Old::Value(1), "high"),
    row(H2.0, H2.1, "light", "diffuse contrast", Old::Value(2), "low"),
    row(H2.0, H2.1, "light", "diffuse contrast", Old::Value(3), "very low"),
    row(H2.0, H2.1, "light", "interpolation flags", Old::Bit(0x1), "blend in hsv"),
    row(H2.0, H2.1, "light", "interpolation flags", Old::Bit(0x2), "...more colors"),
    row(H2.0, H2.1, "biped", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "biped", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "biped", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "biped", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "biped", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "vehicle", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "vehicle", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "vehicle", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "vehicle", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "vehicle", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "weapon", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "weapon", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "weapon", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "weapon", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "weapon", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "scenery", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "scenery", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "scenery", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "scenery", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "scenery", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "projectile", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "projectile", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "projectile", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "projectile", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "projectile", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "device_machine", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "device_machine", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "device_machine", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "device_machine", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "device_machine", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "device_control", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "device_control", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "device_control", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "device_control", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "device_control", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "crate", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "crate", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "crate", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "crate", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "crate", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "creature", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "creature", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "creature", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "creature", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "creature", "attachments/change color", Old::Value(4), "quaternary"),
    row(H2.0, H2.1, "sound_scenery", "attachments/change color", Old::Value(0), "none"),
    row(H2.0, H2.1, "sound_scenery", "attachments/change color", Old::Value(1), "primary"),
    row(H2.0, H2.1, "sound_scenery", "attachments/change color", Old::Value(2), "secondary"),
    row(H2.0, H2.1, "sound_scenery", "attachments/change color", Old::Value(3), "tertiary"),
    row(H2.0, H2.1, "sound_scenery", "attachments/change color", Old::Value(4), "quaternary"),
    // h2::weapons: every game's animation graphs
    row(H2.0, H2.1, "model_animation_graph", "skeleton nodes/model flags", Old::Bit(0x20), "left arm member"),
    row(H2.0, H2.1, "model_animation_graph", "animations/playback flags", Old::Bit(0x8), "disable weapon ik"),
    row(H3.0, H3.1, "model_animation_graph", "skeleton nodes/model flags", Old::Bit(0x20), "left arm member"),
    row(H3.0, H3.1, "model_animation_graph", "animations/playback flags", Old::Bit(0x8), "disable weapon ik"),
    row(ODST.0, ODST.1, "model_animation_graph", "skeleton nodes/model flags", Old::Bit(0x20), "left arm member"),
    row(ODST.0, ODST.1, "model_animation_graph", "animations/playback flags", Old::Bit(0x8), "disable weapon ik"),
    row(REACH.0, REACH.1, "model_animation_graph", "skeleton nodes/model flags", Old::Bit(0x20), "left arm member"),
    row(REACH.0, REACH.1, "model_animation_graph", "animations/user flags", Old::Bit(0x8), "disable weapon ik"),
    row(H4.0, H4.1, "model_animation_graph", "skeleton nodes/model flags", Old::Bit(0x20), "left arm member"),
    row(H4.0, H4.1, "model_animation_graph", "animations/user flags", Old::Bit(0x8), "disable weapon ik"),
    // h3::lights: Halo 3, ODST and Reach
    labelled(H3.0, H3.1, "light", "flags", Old::Bit(0x1), "allow shadows and gels", &["allow shadows", "expensive light"]),
    row(H3.0, H3.1, "light", "flags", Old::Bit(0x4), "render first person only"),
    row(H3.0, H3.1, "light", "type", Old::Value(1), "frustum"),
    row(H3.0, H3.1, "lens_flare", "flags", Old::Bit(0x2), "no occlusion test"),
    row(H3.0, H3.1, "lens_flare", "flags", Old::Bit(0x4), "only render in first person"),
    labelled(ODST.0, ODST.1, "light", "flags", Old::Bit(0x1), "allow shadows and gels", &["allow shadows", "expensive light"]),
    row(ODST.0, ODST.1, "light", "flags", Old::Bit(0x4), "render first person only"),
    row(ODST.0, ODST.1, "light", "type", Old::Value(1), "frustum"),
    row(ODST.0, ODST.1, "lens_flare", "flags", Old::Bit(0x2), "no occlusion test"),
    row(ODST.0, ODST.1, "lens_flare", "flags", Old::Bit(0x4), "only render in first person"),
    labelled(REACH.0, REACH.1, "light", "flags", Old::Bit(0x1), "allow shadows and gels", &["allow shadows", "expensive light"]),
    row(REACH.0, REACH.1, "light", "flags", Old::Bit(0x4), "render first person only"),
    row(REACH.0, REACH.1, "light", "type", Old::Value(1), "frustum"),
    row(REACH.0, REACH.1, "lens_flare", "flags", Old::Bit(0x2), "no occlusion test"),
    row(REACH.0, REACH.1, "lens_flare", "flags", Old::Bit(0x4), "only render in first person"),
    // materials: Halo 2 shaders
    row(H2.0, H2.1, "shader", "specular type", Old::Value(0), "none"),
    // gen3::shaders: render method definitions
    row(H3.0, H3.1, "render_method_definition", "flags", Old::Bit(0x1), "use automatic macros"),
    row(H3.0, H3.1, "render_method_definition", "vertex_types/vertex type", Old::Value(0), "world"),
    row(H3.0, H3.1, "render_method_definition", "vertex_types/vertex type", Old::Value(1), "rigid"),
    row(H3.0, H3.1, "render_method_definition", "vertex_types/vertex type", Old::Value(2), "skinned"),
    row(ODST.0, ODST.1, "render_method_definition", "flags", Old::Bit(0x1), "use automatic macros"),
    row(ODST.0, ODST.1, "render_method_definition", "vertex_types/vertex type", Old::Value(0), "world"),
    row(ODST.0, ODST.1, "render_method_definition", "vertex_types/vertex type", Old::Value(1), "rigid"),
    row(ODST.0, ODST.1, "render_method_definition", "vertex_types/vertex type", Old::Value(2), "skinned"),
    row(REACH.0, REACH.1, "render_method_definition", "flags", Old::Bit(0x1), "use automatic macros"),
    row(REACH.0, REACH.1, "render_method_definition", "vertex_types/vertex type", Old::Value(0), "world"),
    row(REACH.0, REACH.1, "render_method_definition", "vertex_types/vertex type", Old::Value(1), "rigid"),
    row(REACH.0, REACH.1, "render_method_definition", "vertex_types/vertex type", Old::Value(2), "skinned"),
    labelled(REACH.0, REACH.1, "render_method_definition", "entry_points/passes/flags", Old::Bit(0x1), "shared pixel shader compilation", &["shared entry point compilation"]),
    row(REACH.0, REACH.1, "render_method_definition", "entry_points/passes/flags", Old::Bit(0x2), "shared VS only care non-default option of depended category"),
    // particles: every game's particle systems
    row(H2.0, H2.1, "effect", "events/particle systems/camera mode", Old::Value(0), "independent of camera mode"),
    row(H3.0, H3.1, "effect", "events/particle systems/camera mode", Old::Value(0), "independent of camera mode"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/camera mode", Old::Value(0), "independent of camera mode"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/camera mode", Old::Value(0), "independent of camera mode"),
    row(H4.0, H4.1, "effect", "events/particle systems/camera mode", Old::Value(0), "independent of camera mode"),
    // particles: names once read by prefix
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["random u mirror"]), "random u mirror"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["random v mirror"]), "random v mirror"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["random starting rotation"]), "random starting rotation"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["intensity affects alpha"]), "intensity affects alpha"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["bitmap authored vertically"]), "bitmap authored vertically"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["tint from lightmap"]), "tint from lightmap"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["fade when viewed edge-on"]), "fade when viewed edge-on"),
    row(H3.0, H3.1, "particle", "appearance flags", Old::Prefix(&["motion blur"]), "motion blur"),
    row(H3.0, H3.1, "particle", "animation flags", Old::Prefix(&["frame animation one shot"]), "frame animation one shot"),
    row(H3.0, H3.1, "particle", "animation flags", Old::Prefix(&["can animate backwards"]), "can animate backwards"),
    row(H3.0, H3.1, "particle", "main flags", Old::Prefix(&["dies on structure collision"]), "dies on structure collision"),
    row(H3.0, H3.1, "particle", "main flags", Old::Prefix(&["dies at rest"]), "dies at rest"),
    row(H3.0, H3.1, "particle", "main flags", Old::Prefix(&["uses cheap shader"]), "uses cheap shader"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["physics"]), "physics"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["physics"]), "physics"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["swarm"]), "swarm"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["swarm"]), "swarm"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["wind"]), "wind"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["wind"]), "wind"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(H3.0, H3.1, "particle_physics", "flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    row(H3.0, H3.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    labelled(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["lod always 1.0", "ignore lod"]), "lod always 1.0", &["ignore lod (render always)", "lod always 1"]),
    row(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["don't render system"]), "don't render system"),
    row(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["force cpu updating"]), "force cpu updating"),
    row(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["force gpu updating"]), "force gpu updating"),
    row(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["override near fade"]), "override near fade (use with caution)"),
    labelled(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles die when effect ends"]), "particles die when effect ends", &["die when orphaned"]),
    row(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles live forever"]), "particles live forever"),
    row(H3.0, H3.1, "effect", "events/particle systems/flags", Old::Prefix(&["disable velocity"]), "disable velocity"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["random u mirror"]), "random u mirror"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["random v mirror"]), "random v mirror"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["random starting rotation"]), "random starting rotation"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["intensity affects alpha"]), "intensity affects alpha"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["bitmap authored vertically"]), "bitmap authored vertically"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["tint from lightmap"]), "tint from lightmap"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["fade when viewed edge-on"]), "fade when viewed edge-on"),
    row(ODST.0, ODST.1, "particle", "appearance flags", Old::Prefix(&["motion blur"]), "motion blur"),
    row(ODST.0, ODST.1, "particle", "animation flags", Old::Prefix(&["frame animation one shot"]), "frame animation one shot"),
    row(ODST.0, ODST.1, "particle", "animation flags", Old::Prefix(&["can animate backwards"]), "can animate backwards"),
    row(ODST.0, ODST.1, "particle", "main flags", Old::Prefix(&["dies on structure collision"]), "dies on structure collision"),
    row(ODST.0, ODST.1, "particle", "main flags", Old::Prefix(&["dies at rest"]), "dies at rest"),
    row(ODST.0, ODST.1, "particle", "main flags", Old::Prefix(&["uses cheap shader"]), "uses cheap shader"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["physics"]), "physics"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["physics"]), "physics"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["swarm"]), "swarm"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["swarm"]), "swarm"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["wind"]), "wind"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["wind"]), "wind"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(ODST.0, ODST.1, "particle_physics", "flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    labelled(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["lod always 1.0", "ignore lod"]), "lod always 1.0", &["ignore lod (render always)", "lod always 1"]),
    row(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["don't render system"]), "don't render system"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["force cpu updating"]), "force cpu updating"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["force gpu updating"]), "force gpu updating"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["override near fade"]), "override near fade (use with caution)"),
    labelled(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles die when effect ends"]), "particles die when effect ends", &["die when orphaned"]),
    row(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles live forever"]), "particles live forever"),
    row(ODST.0, ODST.1, "effect", "events/particle systems/flags", Old::Prefix(&["disable velocity"]), "disable velocity"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["random u mirror"]), "random u mirror"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["random v mirror"]), "random v mirror"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["random starting rotation"]), "random starting rotation"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["intensity affects alpha"]), "intensity affects alpha"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["bitmap authored vertically"]), "bitmap authored vertically"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["tint from lightmap"]), "tint from lightmap"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["fade when viewed edge-on"]), "fade when viewed edge-on"),
    row(REACH.0, REACH.1, "particle", "appearance flags", Old::Prefix(&["motion blur"]), "motion blur"),
    row(REACH.0, REACH.1, "particle", "animation flags", Old::Prefix(&["frame animation one shot"]), "frame animation one shot"),
    row(REACH.0, REACH.1, "particle", "animation flags", Old::Prefix(&["can animate backwards"]), "can animate backwards"),
    row(REACH.0, REACH.1, "particle", "main flags", Old::Prefix(&["dies on structure collision"]), "dies on structure collision"),
    row(REACH.0, REACH.1, "particle", "main flags", Old::Prefix(&["dies at rest"]), "dies at rest"),
    row(REACH.0, REACH.1, "particle", "main flags", Old::Prefix(&["uses cheap shader"]), "uses cheap shader"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["physics"]), "physics"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["physics"]), "physics"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["swarm"]), "swarm"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["swarm"]), "swarm"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["wind"]), "wind"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["wind"]), "wind"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(REACH.0, REACH.1, "particle_physics", "flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/emitters/emitter flags", Old::Prefix(&["volume emitter particle velocities are random"]), "volume emitter particle velocities are random"),
    labelled(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["lod always 1.0", "ignore lod"]), "lod always 1.0", &["ignore lod (render always)", "lod always 1"]),
    row(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["don't render system"]), "don't render system"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["force cpu updating"]), "force cpu updating"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["force gpu updating"]), "force gpu updating"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["override near fade"]), "override near fade (use with caution)"),
    labelled(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles die when effect ends"]), "particles die when effect ends", &["die when orphaned"]),
    row(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles live forever"]), "particles live forever"),
    row(REACH.0, REACH.1, "effect", "events/particle systems/flags", Old::Prefix(&["disable velocity"]), "disable velocity"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["random u mirror"]), "random u mirror"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["random v mirror"]), "random v mirror"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["random starting rotation"]), "random starting rotation"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["intensity affects alpha"]), "intensity affects alpha"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["bitmap authored vertically"]), "bitmap authored vertically"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["tint from lightmap"]), "tint from lightmap"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["fade when viewed edge-on"]), "fade when viewed edge-on"),
    row(H4.0, H4.1, "particle", "appearance flags", Old::Prefix(&["motion blur"]), "motion blur"),
    row(H4.0, H4.1, "particle", "animation flags", Old::Prefix(&["frame animation one shot"]), "frame animation one shot"),
    row(H4.0, H4.1, "particle", "animation flags", Old::Prefix(&["can animate backwards"]), "can animate backwards"),
    row(H4.0, H4.1, "particle", "main flags", Old::Prefix(&["dies on structure collision"]), "dies on structure collision"),
    row(H4.0, H4.1, "particle", "main flags", Old::Prefix(&["dies at rest"]), "dies at rest"),
    row(H4.0, H4.1, "particle", "main flags", Old::Prefix(&["uses cheap shader"]), "uses cheap shader"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["physics"]), "physics"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["physics"]), "physics"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["swarm"]), "swarm"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["swarm"]), "swarm"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["wind"]), "wind"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["wind"]), "wind"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["turbulence"]), "turbulence"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with water"]), "collide with water"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(H4.0, H4.1, "particle_physics", "flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/particle movement/flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    row(H4.0, H4.1, "effect", "events/particle systems/emitters/emitter flags", Old::Prefix(&["volume emitter particle velocities are random"]), "volume emitter particle velocities are random"),
    labelled(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["lod always 1.0", "ignore lod"]), "lod always 1.0", &["ignore lod (render always)", "lod always 1"]),
    row(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["don't render system"]), "don't render system"),
    row(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["force cpu updating"]), "force cpu updating"),
    row(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["force gpu updating"]), "force gpu updating"),
    row(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["override near fade"]), "override near fade (use with caution)"),
    labelled(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles die when effect ends"]), "particles die when effect ends", &["die when orphaned"]),
    row(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles live forever"]), "particles live forever"),
    row(H4.0, H4.1, "effect", "events/particle systems/flags", Old::Prefix(&["disable velocity"]), "disable velocity"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["disable frame blending"]), "disable frame blending"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["random u mirror"]), "random u mirror"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["random v mirror"]), "random v mirror"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["bitmap authored vertically"]), "bitmap authored vertically"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["dies on structure collision"]), "dies on structure collision"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["dies at rest"]), "dies at rest"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["frame animation one shot"]), "frame animation one shot"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["can animate backwards"]), "can animate backwards"),
    row(H2.0, H2.1, "particle", "flags", Old::Prefix(&["select random sequence"]), "select random sequence"),
    row(H2.0, H2.1, "effect", "events/particle systems/flags", Old::Prefix(&["looping particle"]), "looping particle"),
    row(H2.0, H2.1, "effect", "events/particle systems/flags", Old::Prefix(&["persistent particle"]), "persistent particle"),
    row(H2.0, H2.1, "effect", "events/particle systems/flags", Old::Prefix(&["spread between ticks"]), "spread between ticks"),
    row(H2.0, H2.1, "effect", "events/particle systems/flags", Old::Prefix(&["don't render system"]), "don't render system"),
    row(H2.0, H2.1, "effect", "events/particle systems/flags", Old::Prefix(&["particles die when effect ends"]), "particles die when effect ends"),
    row(H2.0, H2.1, "effect", "events/particle systems/flags", Old::Prefix(&["lod always 1.0"]), "lod always 1.0"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["physics"]), "physics"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["swarm"]), "swarm"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["wind"]), "wind"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["collide with structure"]), "collide with structure"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["collide with media"]), "collide with media"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["collide with scenery"]), "collide with scenery"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["collide with vehicles"]), "collide with vehicles"),
    row(H2.0, H2.1, "particle_physics", "flags", Old::Prefix(&["collide with bipeds"]), "collide with bipeds"),
    // Names once read by prefix, before particles
    row(HCE.0, HCE.1, "shader_transparent_chicago", "extra flags", Old::Prefix(&["numeric countdown timer"]), "numeric countdown timer"),
    row(HCE.0, HCE.1, "shader_transparent_chicago_extended", "extra flags", Old::Prefix(&["numeric countdown timer"]), "numeric countdown timer"),
    row(H2.0, H2.1, "contrail", "flags", Old::Prefix(&["no gravity"]), "no gravity"),
    row(H2.0, H2.1, "contrail", "flags", Old::Prefix(&["first point unfaded"]), "first point unfaded"),
    row(H2.0, H2.1, "contrail", "flags", Old::Prefix(&["last point unfaded"]), "last point unfaded"),
    row(H2.0, H2.1, "light_volume", "volumes/flags", Old::Prefix(&["fuzzy"]), "fuzzy"),
    row(H2.0, H2.1, "light_volume", "volumes/flags", Old::Prefix(&["scaled by marker"]), "scaled by marker"),
    row(H2.0, H2.1, "light_volume", "volumes/flags", Old::Prefix(&["not scaled by event duration"]), "not scaled by event duration"),
    row(H2.0, H2.1, "liquid", "arcs/flags", Old::Prefix(&["basis marker-relative"]), "basis marker-relative"),
    row(H2.0, H2.1, "weapon", "flags", Old::Prefix(&["can be dual wielded"]), "can be dual wielded"),
    row(H2.0, H2.1, "weapon", "flags", Old::Prefix(&["can only be dual wielded"]), "can only be dual wielded"),
    row(H2.0, H2.1, "weapon", "flags", Old::Prefix(&["extension of parent"]), "extension of parent"),
    row(H3.0, H3.1, "weapon", "flags", Old::Prefix(&["can be dual wielded"]), "can be dual wielded"),
    row(H3.0, H3.1, "weapon", "flags", Old::Prefix(&["can only be dual wielded"]), "can only be dual wielded"),
    row(H3.0, H3.1, "weapon", "flags", Old::Prefix(&["extension of parent"]), "extension of parent"),
    row(ODST.0, ODST.1, "weapon", "flags", Old::Prefix(&["can be dual wielded"]), "can be dual wielded"),
    row(ODST.0, ODST.1, "weapon", "flags", Old::Prefix(&["can only be dual wielded"]), "can only be dual wielded"),
    row(ODST.0, ODST.1, "weapon", "flags", Old::Prefix(&["extension of parent"]), "extension of parent"),
    row(REACH.0, REACH.1, "weapon", "flags", Old::Prefix(&["can be dual wielded"]), "can be dual wielded"),
    row(REACH.0, REACH.1, "weapon", "flags", Old::Prefix(&["can only be dual wielded"]), "can only be dual wielded"),
    row(REACH.0, REACH.1, "weapon", "flags", Old::Prefix(&["extension of parent"]), "extension of parent"),
    row(H4.0, H4.1, "weapon", "flags", Old::Prefix(&["can be dual wielded"]), "can be dual wielded"),
    row(H4.0, H4.1, "weapon", "flags", Old::Prefix(&["can only be dual wielded"]), "can only be dual wielded"),
    row(H4.0, H4.1, "weapon", "flags", Old::Prefix(&["extension of parent"]), "extension of parent"),
    row(ODST.0, ODST.1, "shield_impact", "flags", Old::Prefix(&["use secondary"]), "use secondary (non-uniform) texture scale"),
    row(ODST.0, ODST.1, "shield_impact", "flags", Old::Prefix(&["use procedural uv"]), "use procedural uv generation"),
    row(H3.0, H3.1, "lens_flare", "reflections/flags", Old::Prefix(&["rotate from center of screen"]), "rotate from center of screen"),
    row(H3.0, H3.1, "lens_flare", "reflections/flags", Old::Prefix(&["radius scaled by distance"]), "radius scaled by distance"),
    row(H3.0, H3.1, "lens_flare", "reflections/flags", Old::Prefix(&["radius scaled by occlusion factor"]), "radius scaled by occlusion factor"),
    row(H3.0, H3.1, "lens_flare", "reflections/flags", Old::Prefix(&["ignore external color"]), "ignore external color"),
    row(ODST.0, ODST.1, "lens_flare", "reflections/flags", Old::Prefix(&["rotate from center of screen"]), "rotate from center of screen"),
    row(ODST.0, ODST.1, "lens_flare", "reflections/flags", Old::Prefix(&["radius scaled by distance"]), "radius scaled by distance"),
    row(ODST.0, ODST.1, "lens_flare", "reflections/flags", Old::Prefix(&["radius scaled by occlusion factor"]), "radius scaled by occlusion factor"),
    row(ODST.0, ODST.1, "lens_flare", "reflections/flags", Old::Prefix(&["ignore external color"]), "ignore external color"),
    row(REACH.0, REACH.1, "lens_flare", "reflections/flags", Old::Prefix(&["rotate from center of screen"]), "rotate from center of screen"),
    row(REACH.0, REACH.1, "lens_flare", "reflections/flags", Old::Prefix(&["radius scaled by distance"]), "radius scaled by distance"),
    row(REACH.0, REACH.1, "lens_flare", "reflections/flags", Old::Prefix(&["radius scaled by occlusion factor"]), "radius scaled by occlusion factor"),
    row(REACH.0, REACH.1, "lens_flare", "reflections/flags", Old::Prefix(&["ignore external color"]), "ignore external color"),
    // common::attached: Reach and Halo 4 object attachments
    row(REACH.0, REACH.1, "biped", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(REACH.0, REACH.1, "biped", "attachments/change color", Old::Value(0), "none"),
    row(REACH.0, REACH.1, "biped", "attachments/change color", Old::Value(1), "primary"),
    row(REACH.0, REACH.1, "biped", "attachments/change color", Old::Value(2), "secondary"),
    row(REACH.0, REACH.1, "biped", "attachments/change color", Old::Value(3), "tertiary"),
    row(REACH.0, REACH.1, "biped", "attachments/change color", Old::Value(4), "quaternary"),
    row(REACH.0, REACH.1, "vehicle", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(REACH.0, REACH.1, "vehicle", "attachments/change color", Old::Value(0), "none"),
    row(REACH.0, REACH.1, "vehicle", "attachments/change color", Old::Value(1), "primary"),
    row(REACH.0, REACH.1, "vehicle", "attachments/change color", Old::Value(2), "secondary"),
    row(REACH.0, REACH.1, "vehicle", "attachments/change color", Old::Value(3), "tertiary"),
    row(REACH.0, REACH.1, "vehicle", "attachments/change color", Old::Value(4), "quaternary"),
    row(REACH.0, REACH.1, "weapon", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(REACH.0, REACH.1, "weapon", "attachments/change color", Old::Value(0), "none"),
    row(REACH.0, REACH.1, "weapon", "attachments/change color", Old::Value(1), "primary"),
    row(REACH.0, REACH.1, "weapon", "attachments/change color", Old::Value(2), "secondary"),
    row(REACH.0, REACH.1, "weapon", "attachments/change color", Old::Value(3), "tertiary"),
    row(REACH.0, REACH.1, "weapon", "attachments/change color", Old::Value(4), "quaternary"),
    row(REACH.0, REACH.1, "scenery", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(REACH.0, REACH.1, "scenery", "attachments/change color", Old::Value(0), "none"),
    row(REACH.0, REACH.1, "scenery", "attachments/change color", Old::Value(1), "primary"),
    row(REACH.0, REACH.1, "scenery", "attachments/change color", Old::Value(2), "secondary"),
    row(REACH.0, REACH.1, "scenery", "attachments/change color", Old::Value(3), "tertiary"),
    row(REACH.0, REACH.1, "scenery", "attachments/change color", Old::Value(4), "quaternary"),
    row(REACH.0, REACH.1, "crate", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(REACH.0, REACH.1, "crate", "attachments/change color", Old::Value(0), "none"),
    row(REACH.0, REACH.1, "crate", "attachments/change color", Old::Value(1), "primary"),
    row(REACH.0, REACH.1, "crate", "attachments/change color", Old::Value(2), "secondary"),
    row(REACH.0, REACH.1, "crate", "attachments/change color", Old::Value(3), "tertiary"),
    row(REACH.0, REACH.1, "crate", "attachments/change color", Old::Value(4), "quaternary"),
    row(REACH.0, REACH.1, "device_machine", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(REACH.0, REACH.1, "device_machine", "attachments/change color", Old::Value(0), "none"),
    row(REACH.0, REACH.1, "device_machine", "attachments/change color", Old::Value(1), "primary"),
    row(REACH.0, REACH.1, "device_machine", "attachments/change color", Old::Value(2), "secondary"),
    row(REACH.0, REACH.1, "device_machine", "attachments/change color", Old::Value(3), "tertiary"),
    row(REACH.0, REACH.1, "device_machine", "attachments/change color", Old::Value(4), "quaternary"),
    row(H4.0, H4.1, "biped", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(H4.0, H4.1, "biped", "attachments/change color", Old::Value(0), "none"),
    row(H4.0, H4.1, "biped", "attachments/change color", Old::Value(1), "primary"),
    row(H4.0, H4.1, "biped", "attachments/change color", Old::Value(2), "secondary"),
    row(H4.0, H4.1, "biped", "attachments/change color", Old::Value(3), "tertiary"),
    row(H4.0, H4.1, "biped", "attachments/change color", Old::Value(4), "quaternary"),
    row(H4.0, H4.1, "vehicle", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(H4.0, H4.1, "vehicle", "attachments/change color", Old::Value(0), "none"),
    row(H4.0, H4.1, "vehicle", "attachments/change color", Old::Value(1), "primary"),
    row(H4.0, H4.1, "vehicle", "attachments/change color", Old::Value(2), "secondary"),
    row(H4.0, H4.1, "vehicle", "attachments/change color", Old::Value(3), "tertiary"),
    row(H4.0, H4.1, "vehicle", "attachments/change color", Old::Value(4), "quaternary"),
    row(H4.0, H4.1, "weapon", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(H4.0, H4.1, "weapon", "attachments/change color", Old::Value(0), "none"),
    row(H4.0, H4.1, "weapon", "attachments/change color", Old::Value(1), "primary"),
    row(H4.0, H4.1, "weapon", "attachments/change color", Old::Value(2), "secondary"),
    row(H4.0, H4.1, "weapon", "attachments/change color", Old::Value(3), "tertiary"),
    row(H4.0, H4.1, "weapon", "attachments/change color", Old::Value(4), "quaternary"),
    row(H4.0, H4.1, "scenery", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(H4.0, H4.1, "scenery", "attachments/change color", Old::Value(0), "none"),
    row(H4.0, H4.1, "scenery", "attachments/change color", Old::Value(1), "primary"),
    row(H4.0, H4.1, "scenery", "attachments/change color", Old::Value(2), "secondary"),
    row(H4.0, H4.1, "scenery", "attachments/change color", Old::Value(3), "tertiary"),
    row(H4.0, H4.1, "scenery", "attachments/change color", Old::Value(4), "quaternary"),
    row(H4.0, H4.1, "crate", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(H4.0, H4.1, "crate", "attachments/change color", Old::Value(0), "none"),
    row(H4.0, H4.1, "crate", "attachments/change color", Old::Value(1), "primary"),
    row(H4.0, H4.1, "crate", "attachments/change color", Old::Value(2), "secondary"),
    row(H4.0, H4.1, "crate", "attachments/change color", Old::Value(3), "tertiary"),
    row(H4.0, H4.1, "crate", "attachments/change color", Old::Value(4), "quaternary"),
    row(H4.0, H4.1, "device_machine", "attachments/flags", Old::Prefix(&["force always on"]), "force always on"),
    row(H4.0, H4.1, "device_machine", "attachments/change color", Old::Value(0), "none"),
    row(H4.0, H4.1, "device_machine", "attachments/change color", Old::Value(1), "primary"),
    row(H4.0, H4.1, "device_machine", "attachments/change color", Old::Value(2), "secondary"),
    row(H4.0, H4.1, "device_machine", "attachments/change color", Old::Value(3), "tertiary"),
    row(H4.0, H4.1, "device_machine", "attachments/change color", Old::Value(4), "quaternary"),
    // particles::cheap: Reach and Halo 4
    row(REACH.0, REACH.1, "cheap_particle_emitter", "flags", Old::Prefix(&["correlate position and velocity"]), "correlate position and velocity"),
    row(REACH.0, REACH.1, "cheap_particle_emitter", "flags", Old::Prefix(&["position texture in local space"]), "position texture in local space"),
    row(REACH.0, REACH.1, "cheap_particle_emitter", "flags", Old::Prefix(&["velocity texture in local space"]), "velocity texture in local space"),
    row(REACH.0, REACH.1, "cheap_particle_emitter", "flags", Old::Prefix(&["normalize velocity before scaling"]), "normalize velocity before scaling"),
    row(REACH.0, REACH.1, "cheap_particle_emitter", "flags", Old::Prefix(&["randomly rotate emitter about up vector"]), "randomly rotate emitter about up vector"),
    row(REACH.0, REACH.1, "cheap_particle_type_library", "types/orientation", Old::Value(0), "velocity"),
    row(H4.0, H4.1, "cheap_particle_emitter", "flags", Old::Prefix(&["correlate position and velocity"]), "correlate position and velocity"),
    row(H4.0, H4.1, "cheap_particle_emitter", "flags", Old::Prefix(&["position texture in local space"]), "position texture in local space"),
    row(H4.0, H4.1, "cheap_particle_emitter", "flags", Old::Prefix(&["velocity texture in local space"]), "velocity texture in local space"),
    row(H4.0, H4.1, "cheap_particle_emitter", "flags", Old::Prefix(&["normalize velocity before scaling"]), "normalize velocity before scaling"),
    row(H4.0, H4.1, "cheap_particle_emitter", "flags", Old::Prefix(&["randomly rotate emitter about up vector"]), "randomly rotate emitter about up vector"),
    row(H4.0, H4.1, "cheap_particle_type_library", "types/orientation", Old::Value(0), "velocity"),
    // gen3::effects: every game from Halo 2
    row(H2.0, H2.1, "effect", "flags", Old::Prefix(&["run events in parallel"]), "run events in parallel"),
    row(H2.0, H2.1, "effect", "flags", Old::Prefix(&["use parent position but world orientation"]), "use parent position but world orientation"),
    row(H2.0, H2.1, "effect", "events/parts/flags", Old::Prefix(&["disabled for debugging"]), "disabled for debugging"),
    row(H2.0, H2.1, "effect", "events/parts/flags", Old::Prefix(&["never attached to object"]), "never attached to object"),
    row(H2.0, H2.1, "effect", "events/parts/A scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(H2.0, H2.1, "effect", "events/parts/B scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(H3.0, H3.1, "effect", "flags", Old::Prefix(&["run events in parallel"]), "run events in parallel"),
    row(H3.0, H3.1, "effect", "flags", Old::Prefix(&["use parent position but world orientation"]), "use parent position but world orientation"),
    row(H3.0, H3.1, "effect", "events/parts/flags", Old::Prefix(&["disabled for debugging"]), "disabled for debugging"),
    row(H3.0, H3.1, "effect", "events/parts/flags", Old::Prefix(&["never attached to object"]), "never attached to object"),
    row(H3.0, H3.1, "effect", "events/parts/A scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(H3.0, H3.1, "effect", "events/parts/B scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(ODST.0, ODST.1, "effect", "flags", Old::Prefix(&["run events in parallel"]), "run events in parallel"),
    row(ODST.0, ODST.1, "effect", "flags", Old::Prefix(&["use parent position but world orientation"]), "use parent position but world orientation"),
    row(ODST.0, ODST.1, "effect", "events/parts/flags", Old::Prefix(&["disabled for debugging"]), "disabled for debugging"),
    row(ODST.0, ODST.1, "effect", "events/parts/flags", Old::Prefix(&["never attached to object"]), "never attached to object"),
    row(ODST.0, ODST.1, "effect", "events/parts/A scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(ODST.0, ODST.1, "effect", "events/parts/B scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(REACH.0, REACH.1, "effect", "flags", Old::Prefix(&["run events in parallel"]), "run events in parallel"),
    row(REACH.0, REACH.1, "effect", "flags", Old::Prefix(&["use parent position but world orientation"]), "use parent position but world orientation"),
    row(REACH.0, REACH.1, "effect", "events/parts/flags", Old::Prefix(&["disabled for debugging"]), "disabled for debugging"),
    row(REACH.0, REACH.1, "effect", "events/parts/flags", Old::Prefix(&["never attached to object"]), "never attached to object"),
    row(REACH.0, REACH.1, "effect", "events/parts/A scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(REACH.0, REACH.1, "effect", "events/parts/B scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(H4.0, H4.1, "effect", "flags", Old::Prefix(&["run events in parallel"]), "run events in parallel"),
    row(H4.0, H4.1, "effect", "flags", Old::Prefix(&["use parent position but world orientation"]), "use parent position but world orientation"),
    row(H4.0, H4.1, "effect", "events/parts/flags", Old::Prefix(&["disabled for debugging"]), "disabled for debugging"),
    row(H4.0, H4.1, "effect", "events/parts/flags", Old::Prefix(&["never attached to object"]), "never attached to object"),
    row(H4.0, H4.1, "effect", "events/parts/A scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    row(H4.0, H4.1, "effect", "events/parts/B scales values", Old::Prefix(&["type-specific scale"]), "type-specific scale"),
    // gen3::effects and h3::lights: Halo 3 and ODST object attachments
    row(H3.0, H3.1, "biped", "attachments/change color", Old::Value(0), "none"),
    row(H3.0, H3.1, "biped", "attachments/change color", Old::Value(1), "primary"),
    row(H3.0, H3.1, "biped", "attachments/change color", Old::Value(2), "secondary"),
    row(H3.0, H3.1, "biped", "attachments/change color", Old::Value(3), "tertiary"),
    row(H3.0, H3.1, "biped", "attachments/change color", Old::Value(4), "quaternary"),
    row(H3.0, H3.1, "crate", "attachments/change color", Old::Value(0), "none"),
    row(H3.0, H3.1, "crate", "attachments/change color", Old::Value(1), "primary"),
    row(H3.0, H3.1, "crate", "attachments/change color", Old::Value(2), "secondary"),
    row(H3.0, H3.1, "crate", "attachments/change color", Old::Value(3), "tertiary"),
    row(H3.0, H3.1, "crate", "attachments/change color", Old::Value(4), "quaternary"),
    row(H3.0, H3.1, "device_machine", "attachments/change color", Old::Value(0), "none"),
    row(H3.0, H3.1, "device_machine", "attachments/change color", Old::Value(1), "primary"),
    row(H3.0, H3.1, "device_machine", "attachments/change color", Old::Value(2), "secondary"),
    row(H3.0, H3.1, "device_machine", "attachments/change color", Old::Value(3), "tertiary"),
    row(H3.0, H3.1, "device_machine", "attachments/change color", Old::Value(4), "quaternary"),
    row(H3.0, H3.1, "scenery", "attachments/change color", Old::Value(0), "none"),
    row(H3.0, H3.1, "scenery", "attachments/change color", Old::Value(1), "primary"),
    row(H3.0, H3.1, "scenery", "attachments/change color", Old::Value(2), "secondary"),
    row(H3.0, H3.1, "scenery", "attachments/change color", Old::Value(3), "tertiary"),
    row(H3.0, H3.1, "scenery", "attachments/change color", Old::Value(4), "quaternary"),
    row(H3.0, H3.1, "vehicle", "attachments/change color", Old::Value(0), "none"),
    row(H3.0, H3.1, "vehicle", "attachments/change color", Old::Value(1), "primary"),
    row(H3.0, H3.1, "vehicle", "attachments/change color", Old::Value(2), "secondary"),
    row(H3.0, H3.1, "vehicle", "attachments/change color", Old::Value(3), "tertiary"),
    row(H3.0, H3.1, "vehicle", "attachments/change color", Old::Value(4), "quaternary"),
    row(H3.0, H3.1, "weapon", "attachments/change color", Old::Value(0), "none"),
    row(H3.0, H3.1, "weapon", "attachments/change color", Old::Value(1), "primary"),
    row(H3.0, H3.1, "weapon", "attachments/change color", Old::Value(2), "secondary"),
    row(H3.0, H3.1, "weapon", "attachments/change color", Old::Value(3), "tertiary"),
    row(H3.0, H3.1, "weapon", "attachments/change color", Old::Value(4), "quaternary"),
    row(ODST.0, ODST.1, "biped", "attachments/change color", Old::Value(0), "none"),
    row(ODST.0, ODST.1, "biped", "attachments/change color", Old::Value(1), "primary"),
    row(ODST.0, ODST.1, "biped", "attachments/change color", Old::Value(2), "secondary"),
    row(ODST.0, ODST.1, "biped", "attachments/change color", Old::Value(3), "tertiary"),
    row(ODST.0, ODST.1, "biped", "attachments/change color", Old::Value(4), "quaternary"),
    row(ODST.0, ODST.1, "crate", "attachments/change color", Old::Value(0), "none"),
    row(ODST.0, ODST.1, "crate", "attachments/change color", Old::Value(1), "primary"),
    row(ODST.0, ODST.1, "crate", "attachments/change color", Old::Value(2), "secondary"),
    row(ODST.0, ODST.1, "crate", "attachments/change color", Old::Value(3), "tertiary"),
    row(ODST.0, ODST.1, "crate", "attachments/change color", Old::Value(4), "quaternary"),
    row(ODST.0, ODST.1, "device_machine", "attachments/change color", Old::Value(0), "none"),
    row(ODST.0, ODST.1, "device_machine", "attachments/change color", Old::Value(1), "primary"),
    row(ODST.0, ODST.1, "device_machine", "attachments/change color", Old::Value(2), "secondary"),
    row(ODST.0, ODST.1, "device_machine", "attachments/change color", Old::Value(3), "tertiary"),
    row(ODST.0, ODST.1, "device_machine", "attachments/change color", Old::Value(4), "quaternary"),
    row(ODST.0, ODST.1, "scenery", "attachments/change color", Old::Value(0), "none"),
    row(ODST.0, ODST.1, "scenery", "attachments/change color", Old::Value(1), "primary"),
    row(ODST.0, ODST.1, "scenery", "attachments/change color", Old::Value(2), "secondary"),
    row(ODST.0, ODST.1, "scenery", "attachments/change color", Old::Value(3), "tertiary"),
    row(ODST.0, ODST.1, "scenery", "attachments/change color", Old::Value(4), "quaternary"),
    row(ODST.0, ODST.1, "vehicle", "attachments/change color", Old::Value(0), "none"),
    row(ODST.0, ODST.1, "vehicle", "attachments/change color", Old::Value(1), "primary"),
    row(ODST.0, ODST.1, "vehicle", "attachments/change color", Old::Value(2), "secondary"),
    row(ODST.0, ODST.1, "vehicle", "attachments/change color", Old::Value(3), "tertiary"),
    row(ODST.0, ODST.1, "vehicle", "attachments/change color", Old::Value(4), "quaternary"),
    row(ODST.0, ODST.1, "weapon", "attachments/change color", Old::Value(0), "none"),
    row(ODST.0, ODST.1, "weapon", "attachments/change color", Old::Value(1), "primary"),
    row(ODST.0, ODST.1, "weapon", "attachments/change color", Old::Value(2), "secondary"),
    row(ODST.0, ODST.1, "weapon", "attachments/change color", Old::Value(3), "tertiary"),
    row(ODST.0, ODST.1, "weapon", "attachments/change color", Old::Value(4), "quaternary"),
    // gen3::strips: Halo 3, ODST and Reach. Halo 3's and ODST's tag layouts
    // number their input variables differently from one another, so only
    // Reach's have a by-number row.
    row(H3.0, H3.1, "beam_system", "beams/appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(H3.0, H3.1, "beam_system", "beams/profile shape", Old::Value(1), "cross"),
    row(H3.0, H3.1, "beam_system", "beams/profile shape", Old::Value(2), "n-gon"),
    row(H3.0, H3.1, "beam_system", "beams/length/Output Modifier", Old::Value(1), "Plus"),
    row(H3.0, H3.1, "beam_system", "beams/length/Output Modifier", Old::Value(2), "Times"),
    row(H3.0, H3.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["tint from lightmap"]), "tint from lightmap"),
    row(H3.0, H3.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(H3.0, H3.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["profile opacity from scale a"]), "profile opacity from scale a"),
    row(H3.0, H3.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["random u offset"]), "random u offset"),
    row(H3.0, H3.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["random v offset"]), "random v offset"),
    row(H3.0, H3.1, "contrail_system", "contrails/profile shape", Old::Value(1), "cross"),
    row(H3.0, H3.1, "contrail_system", "contrails/profile shape", Old::Value(2), "n-gon"),
    row(H3.0, H3.1, "contrail_system", "contrails/emission rate/Output Modifier", Old::Value(1), "Plus"),
    row(H3.0, H3.1, "contrail_system", "contrails/emission rate/Output Modifier", Old::Value(2), "Times"),
    row(ODST.0, ODST.1, "beam_system", "beams/appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(ODST.0, ODST.1, "beam_system", "beams/profile shape", Old::Value(1), "cross"),
    row(ODST.0, ODST.1, "beam_system", "beams/profile shape", Old::Value(2), "n-gon"),
    row(ODST.0, ODST.1, "beam_system", "beams/length/Output Modifier", Old::Value(1), "Plus"),
    row(ODST.0, ODST.1, "beam_system", "beams/length/Output Modifier", Old::Value(2), "Times"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["tint from lightmap"]), "tint from lightmap"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["profile opacity from scale a"]), "profile opacity from scale a"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["random u offset"]), "random u offset"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["random v offset"]), "random v offset"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/profile shape", Old::Value(1), "cross"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/profile shape", Old::Value(2), "n-gon"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/emission rate/Output Modifier", Old::Value(1), "Plus"),
    row(ODST.0, ODST.1, "contrail_system", "contrails/emission rate/Output Modifier", Old::Value(2), "Times"),
    row(REACH.0, REACH.1, "beam_system", "beams/appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(REACH.0, REACH.1, "beam_system", "beams/profile shape", Old::Value(1), "cross"),
    row(REACH.0, REACH.1, "beam_system", "beams/profile shape", Old::Value(2), "n-gon"),
    row(REACH.0, REACH.1, "beam_system", "beams/length/Output Modifier", Old::Value(1), "Plus"),
    row(REACH.0, REACH.1, "beam_system", "beams/length/Output Modifier", Old::Value(2), "Times"),
    row(REACH.0, REACH.1, "beam_system", "beams/length/Input Variable", Old::Value(3), "beam age"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["tint from lightmap"]), "tint from lightmap"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["double-sided"]), "double-sided"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["profile opacity from scale a"]), "profile opacity from scale a"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["random u offset"]), "random u offset"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/appearance flags", Old::Prefix(&["random v offset"]), "random v offset"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/profile shape", Old::Value(1), "cross"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/profile shape", Old::Value(2), "n-gon"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/emission rate/Output Modifier", Old::Value(1), "Plus"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/emission rate/Output Modifier", Old::Value(2), "Times"),
    row(REACH.0, REACH.1, "contrail_system", "contrails/emission rate/Input Variable", Old::Value(7), "contrail age"),
    // gen3::light_volumes
    row(REACH.0, REACH.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["lod enabled"]), "lod enabled"),
    row(REACH.0, REACH.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["disable precompiled profiles"]), "disable precompiled profiles"),
    row(REACH.0, REACH.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["force precompile profiles"]), "force precompile profiles"),
    row(REACH.0, REACH.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["lod always 1.0", "ignore lod"]), "lod always 1.0"),
    row(H4.0, H4.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["lod enabled"]), "lod enabled"),
    row(H4.0, H4.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["disable precompiled profiles"]), "disable precompiled profiles"),
    row(H4.0, H4.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["force precompile profiles"]), "force precompile profiles"),
    row(H4.0, H4.1, "light_volume_system", "light_volumes/flags", Old::Prefix(&["lod always 1.0", "ignore lod"]), "lod always 1.0"),
    row(H3.0, H3.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(1), "Plus"),
    row(H3.0, H3.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(2), "Times"),
    row(ODST.0, ODST.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(1), "Plus"),
    row(ODST.0, ODST.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(2), "Times"),
    row(REACH.0, REACH.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(1), "Plus"),
    row(REACH.0, REACH.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(2), "Times"),
    row(H4.0, H4.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(1), "Plus"),
    row(H4.0, H4.1, "light_volume_system", "light_volumes/length/Output Modifier", Old::Value(2), "Times"),
    // gen3::screen_effects
    row(H3.0, H3.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["debug disable"]), "debug disable"),
    row(H3.0, H3.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["allow effect outside radius"]), "allow effect outside radius"),
    row(H3.0, H3.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["unattached"]), "unattached"),
    labelled(H3.0, H3.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["first person"]), "first person", &["first person only"]),
    labelled(H3.0, H3.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["third person"]), "third person", &["third person only"]),
    labelled(H3.0, H3.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["disable camera falloffs"]), "disable camera falloffs", &["cinematic"]),
    row(ODST.0, ODST.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["debug disable"]), "debug disable"),
    row(ODST.0, ODST.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["allow effect outside radius"]), "allow effect outside radius"),
    row(ODST.0, ODST.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["unattached"]), "unattached"),
    labelled(ODST.0, ODST.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["first person"]), "first person", &["first person only"]),
    labelled(ODST.0, ODST.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["third person"]), "third person", &["third person only"]),
    labelled(ODST.0, ODST.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["disable camera falloffs"]), "disable camera falloffs", &["cinematic"]),
    row(REACH.0, REACH.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["debug disable"]), "debug disable"),
    row(REACH.0, REACH.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["allow effect outside radius"]), "allow effect outside radius"),
    row(REACH.0, REACH.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["unattached"]), "unattached"),
    labelled(REACH.0, REACH.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["first person"]), "first person", &["first person only"]),
    labelled(REACH.0, REACH.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["third person"]), "third person", &["third person only"]),
    labelled(REACH.0, REACH.1, "area_screen_effect", "screen effects/flags", Old::Prefix(&["disable camera falloffs"]), "disable camera falloffs", &["cinematic"]),
    row(ODST.0, ODST.1, "area_screen_effect", "global flags", Old::Prefix(&["play sequentially"]), "play sequentially - ignore delay settings"),
    row(REACH.0, REACH.1, "area_screen_effect", "global flags", Old::Prefix(&["play sequentially"]), "play sequentially - ignore delay settings"),
    // cloth, h3::vision and variants
    row(H2.0, H2.1, "cloth", "flags", Old::Prefix(&["Uses grid attach top"]), "Uses grid attach top"),
    row(H3.0, H3.1, "cloth", "flags", Old::Prefix(&["Uses grid attach top"]), "Uses grid attach top"),
    row(ODST.0, ODST.1, "cloth", "flags", Old::Prefix(&["Uses grid attach top"]), "Uses grid attach top"),
    row(REACH.0, REACH.1, "cloth", "flags", Old::Prefix(&["Uses grid attach top"]), "Uses grid attach top"),
    row(H4.0, H4.1, "cloth", "flags", Old::Prefix(&["Uses grid attach top"]), "Uses grid attach top"),
    row(H3.0, H3.1, "biped", "unit/default team", Old::Value(1), "player"),
    row(H3.0, H3.1, "creature", "default team", Old::Value(1), "player"),
    row(ODST.0, ODST.1, "biped", "unit/default team", Old::Value(1), "player"),
    row(ODST.0, ODST.1, "creature", "default team", Old::Value(1), "player"),
    row(H3.0, H3.1, "model", "instance groups/choice", Old::Value(1), "choose all members"),
    row(ODST.0, ODST.1, "model", "instance groups/choice", Old::Value(1), "choose all members"),
    row(REACH.0, REACH.1, "model", "instance groups/choice", Old::Value(1), "choose all members"),
    row(H4.0, H4.1, "model", "instance groups/choice", Old::Value(1), "choose all members"),
    // ce::particles: effects
    row(HCE.0, HCE.1, "effect", "flags", Old::Prefix(&["deleted when attachment deactivates"]), "deleted when attachment deactivates"),
    row(HCE.0, HCE.1, "effect", "events/particles/create", Old::Value(1), "only in first person"),
    row(HCE.0, HCE.1, "effect", "events/parts/create in", Old::Value(0), "any environment"),
    row(HCE.0, HCE.1, "effect", "events/parts/create in", Old::Value(1), "air only"),
    row(HCE.0, HCE.1, "effect", "events/parts/create in", Old::Value(2), "water only"),
    row(HCE.0, HCE.1, "effect", "events/parts/create in", Old::Value(3), "space only"),
    row(HCE.0, HCE.1, "effect", "events/parts/violence mode", Old::Value(0), "either mode"),
    row(HCE.0, HCE.1, "effect", "events/parts/violence mode", Old::Value(1), "violent mode only"),
    row(HCE.0, HCE.1, "effect", "events/parts/violence mode", Old::Value(2), "nonviolent mode only"),
    row(HCE.0, HCE.1, "effect", "events/particles/create in", Old::Value(0), "any environment"),
    row(HCE.0, HCE.1, "effect", "events/particles/create in", Old::Value(1), "air only"),
    row(HCE.0, HCE.1, "effect", "events/particles/create in", Old::Value(2), "water only"),
    row(HCE.0, HCE.1, "effect", "events/particles/create in", Old::Value(3), "space only"),
    row(HCE.0, HCE.1, "effect", "events/particles/violence mode", Old::Value(0), "either mode"),
    row(HCE.0, HCE.1, "effect", "events/particles/violence mode", Old::Value(1), "violent mode only"),
    row(HCE.0, HCE.1, "effect", "events/particles/violence mode", Old::Value(2), "nonviolent mode only"),
    // reach::shield, reach::shadow and h4::shadow
    row(REACH.0, REACH.1, "shield_impact", "flags", Old::Prefix(&["use procedural uv generation"]), "use procedural uv generation"),
    row(REACH.0, REACH.1, "shield_impact", "flags", Old::Prefix(&["render always"]), "render always"),
    row(REACH.0, REACH.1, "model", "damage info/damage sections/flags", Old::Prefix(&["starts inactive"]), "starts inactive"),
    row(REACH.0, REACH.1, "biped", "unit/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(REACH.0, REACH.1, "biped", "unit/object/lightmap shadow mode", Old::Value(0), "default"),
    row(REACH.0, REACH.1, "biped", "unit/object/lightmap shadow mode", Old::Value(1), "never"),
    row(REACH.0, REACH.1, "biped", "unit/object/lightmap shadow mode", Old::Value(2), "always"),
    row(REACH.0, REACH.1, "biped", "unit/object/lightmap shadow mode", Old::Value(3), "blur"),
    row(REACH.0, REACH.1, "vehicle", "unit/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(REACH.0, REACH.1, "vehicle", "unit/object/lightmap shadow mode", Old::Value(0), "default"),
    row(REACH.0, REACH.1, "vehicle", "unit/object/lightmap shadow mode", Old::Value(1), "never"),
    row(REACH.0, REACH.1, "vehicle", "unit/object/lightmap shadow mode", Old::Value(2), "always"),
    row(REACH.0, REACH.1, "vehicle", "unit/object/lightmap shadow mode", Old::Value(3), "blur"),
    row(REACH.0, REACH.1, "weapon", "item/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(REACH.0, REACH.1, "weapon", "item/object/lightmap shadow mode", Old::Value(0), "default"),
    row(REACH.0, REACH.1, "weapon", "item/object/lightmap shadow mode", Old::Value(1), "never"),
    row(REACH.0, REACH.1, "weapon", "item/object/lightmap shadow mode", Old::Value(2), "always"),
    row(REACH.0, REACH.1, "weapon", "item/object/lightmap shadow mode", Old::Value(3), "blur"),
    row(REACH.0, REACH.1, "scenery", "object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(REACH.0, REACH.1, "scenery", "object/lightmap shadow mode", Old::Value(0), "default"),
    row(REACH.0, REACH.1, "scenery", "object/lightmap shadow mode", Old::Value(1), "never"),
    row(REACH.0, REACH.1, "scenery", "object/lightmap shadow mode", Old::Value(2), "always"),
    row(REACH.0, REACH.1, "scenery", "object/lightmap shadow mode", Old::Value(3), "blur"),
    row(REACH.0, REACH.1, "crate", "object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(REACH.0, REACH.1, "crate", "object/lightmap shadow mode", Old::Value(0), "default"),
    row(REACH.0, REACH.1, "crate", "object/lightmap shadow mode", Old::Value(1), "never"),
    row(REACH.0, REACH.1, "crate", "object/lightmap shadow mode", Old::Value(2), "always"),
    row(REACH.0, REACH.1, "crate", "object/lightmap shadow mode", Old::Value(3), "blur"),
    row(REACH.0, REACH.1, "device_machine", "device/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(REACH.0, REACH.1, "device_machine", "device/object/lightmap shadow mode", Old::Value(0), "default"),
    row(REACH.0, REACH.1, "device_machine", "device/object/lightmap shadow mode", Old::Value(1), "never"),
    row(REACH.0, REACH.1, "device_machine", "device/object/lightmap shadow mode", Old::Value(2), "always"),
    row(REACH.0, REACH.1, "device_machine", "device/object/lightmap shadow mode", Old::Value(3), "blur"),
    row(H4.0, H4.1, "biped", "unit/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(H4.0, H4.1, "biped", "unit/object/lightmap shadow mode", Old::Value(0), "default"),
    row(H4.0, H4.1, "biped", "unit/object/lightmap shadow mode", Old::Value(1), "never"),
    row(H4.0, H4.1, "biped", "unit/object/lightmap shadow mode", Old::Value(2), "always"),
    row(H4.0, H4.1, "vehicle", "unit/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(H4.0, H4.1, "vehicle", "unit/object/lightmap shadow mode", Old::Value(0), "default"),
    row(H4.0, H4.1, "vehicle", "unit/object/lightmap shadow mode", Old::Value(1), "never"),
    row(H4.0, H4.1, "vehicle", "unit/object/lightmap shadow mode", Old::Value(2), "always"),
    row(H4.0, H4.1, "weapon", "item/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(H4.0, H4.1, "weapon", "item/object/lightmap shadow mode", Old::Value(0), "default"),
    row(H4.0, H4.1, "weapon", "item/object/lightmap shadow mode", Old::Value(1), "never"),
    row(H4.0, H4.1, "weapon", "item/object/lightmap shadow mode", Old::Value(2), "always"),
    row(H4.0, H4.1, "scenery", "object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(H4.0, H4.1, "scenery", "object/lightmap shadow mode", Old::Value(0), "default"),
    row(H4.0, H4.1, "scenery", "object/lightmap shadow mode", Old::Value(1), "never"),
    row(H4.0, H4.1, "scenery", "object/lightmap shadow mode", Old::Value(2), "always"),
    row(H4.0, H4.1, "crate", "object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(H4.0, H4.1, "crate", "object/lightmap shadow mode", Old::Value(0), "default"),
    row(H4.0, H4.1, "crate", "object/lightmap shadow mode", Old::Value(1), "never"),
    row(H4.0, H4.1, "crate", "object/lightmap shadow mode", Old::Value(2), "always"),
    row(H4.0, H4.1, "device_machine", "device/object/flags", Old::Prefix(&["does not cast shadow"]), "does not cast shadow"),
    row(H4.0, H4.1, "device_machine", "device/object/lightmap shadow mode", Old::Value(0), "default"),
    row(H4.0, H4.1, "device_machine", "device/object/lightmap shadow mode", Old::Value(1), "never"),
    row(H4.0, H4.1, "device_machine", "device/object/lightmap shadow mode", Old::Value(2), "always"),
    // h4::tracers and materials::h4
    row(H4.0, H4.1, "tracer_system", "tracer system flags", Old::Prefix(&["point-to-point"]), "point-to-point"),
    row(H4.0, H4.1, "tracer_system", "tracers/flags", Old::Prefix(&["disabled for debugging"]), "disabled for debugging"),
    row(H4.0, H4.1, "tracer_system", "tracers/profile shape", Old::Value(0), "aligned ribbon"),
    row(H4.0, H4.1, "tracer_system", "tracers/profile shape", Old::Value(1), "cross"),
    row(H4.0, H4.1, "tracer_system", "tracers/profile shape", Old::Value(2), "n-gon"),
    row(H4.0, H4.1, "tracer_system", "tracers/profile shape", Old::Value(3), "horizontal"),
    row(H4.0, H4.1, "tracer_system", "tracers/profile shape", Old::Value(4), "vertical"),
    row(H4.0, H4.1, "tracer_system", "tracers/length/Output Modifier", Old::Value(1), "Plus"),
    row(H4.0, H4.1, "tracer_system", "tracers/length/Output Modifier", Old::Value(2), "Times"),
    row(H4.0, H4.1, "tracer_system", "tracers/length/Input Variable", Old::Value(13), "tracer length"),
    row(H4.0, H4.1, "bitmap", "bitmaps/type", Old::Value(2), "cube map"),
    // reach::lights, h4::lights and h4::flares
    row(REACH.0, REACH.1, "light", "flags", Old::Bit(0x400), "screenspace specular lighting"),
    row(REACH.0, REACH.1, "light", "light method", Old::Value(0), "Screenspace"),
    row(REACH.0, REACH.1, "light", "light method", Old::Value(1), "Inline"),
    row(REACH.0, REACH.1, "light", "light method", Old::Value(2), "Re-render"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/flags", Old::Bit(1 << 2), "render first person only"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/flags", Old::Bit(1 << 11), "always on for weapon"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/Light Type", Old::Value(1), "Spot"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/Light Type", Old::Value(2), "Directional"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/Lighting Mode", Old::Value(0), "Physically Correct"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/Screenspace Light", Old::Value(1), "On"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/Specular Contribution", Old::Value(1), "On"),
    row(H4.0, H4.1, "light", "Midnight_Light_Parameters/Diffuse Contribution", Old::Value(1), "On"),
    row(H4.0, H4.1, "cheap_light", "function input", Old::Value(0), "time/age"),
    row(H4.0, H4.1, "lens_flare", "flags", Old::Bit(0x1), "rotate occlusion testing box with lens flare"),
    row(H4.0, H4.1, "lens_flare", "flags", Old::Bit(0x2), "no occlusion test"),
    row(H4.0, H4.1, "lens_flare", "flags", Old::Bit(0x4), "only render in first person"),
    row(H4.0, H4.1, "lens_flare", "flags", Old::Bit(0x10), "use simple occlusion box test"),
    row(H4.0, H4.1, "lens_flare", "flags", Old::Bit(0x20), "no reflection opacity feedback"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x1), "rotate from center of screen"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x2), "radius scaled by distance"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x4), "radius scaled by occlusion factor"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x8), "ignore external color"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x10), "lock to flare X"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x20), "lock to flare Y"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x40), "proximity to center as function input"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x80), "mirror across flare"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x200), "flip U coordinate"),
    row(H4.0, H4.1, "lens_flare", "reflections/flags", Old::Bit(0x400), "flip V coordinate"),
    row(H4.0, H4.1, "lens_flare", "occlusion offset direction", Old::Value(0), "toward viewer"),
    row(H4.0, H4.1, "lens_flare", "occlusion offset direction", Old::Value(1), "marker forward"),
    row(H4.0, H4.1, "lens_flare", "occlusion offset direction", Old::Value(2), "none"),
    row(H4.0, H4.1, "lens_flare", "rotation function", Old::Value(0), "none"),
    row(H4.0, H4.1, "lens_flare", "rotation function", Old::Value(1), "rotation A"),
    row(H4.0, H4.1, "lens_flare", "rotation function", Old::Value(2), "rotation B"),
    row(H4.0, H4.1, "lens_flare", "rotation function", Old::Value(3), "rotation-translation"),
    row(H4.0, H4.1, "lens_flare", "rotation function", Old::Value(4), "translation"),
    // ce::particle_systems
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x1), "type states loop"),
    after(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x2), "type states loop"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x4), "particle states loop"),
    after(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x8), "particle states loop"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x20), "particles die in air"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x40), "particles die on ground"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x80), "rotational sprites animate sideways"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x100), "disabled"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x200), "tint by effect color"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x400), "initial count scales with effect"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x800), "minimum count scales with effect"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x1000), "creation rate scales with effect"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x2000), "scale scales with effect"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x4000), "animation rate scales with effect"),
    row(HCE.0, HCE.1, "particle_system", "particle types/flags", Old::Bit(0x8000), "rotation rate scales with effect"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle states/shader flags", Old::Bit(0x2), "nonlinear tint"),
    row(HCE.0, HCE.1, "particle_system", "particle types/complex sprite render modes", Old::Value(0), "simple"),
    row(HCE.0, HCE.1, "particle_system", "particle types/complex sprite render modes", Old::Value(1), "rotational"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle creation physics", Old::Value(0), "default"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle creation physics", Old::Value(1), "explosion"),
    row(HCE.0, HCE.1, "particle_system", "particle types/particle creation physics", Old::Value(2), "jet"),
    row(HCE.0, HCE.1, "particle_system", "particle types/states/particle creation physics", Old::Value(0), "default"),
    row(HCE.0, HCE.1, "particle_system", "particle types/states/particle creation physics", Old::Value(1), "explosion"),
    row(HCE.0, HCE.1, "particle_system", "particle types/states/particle creation physics", Old::Value(2), "jet"),
    row(HCE.0, HCE.1, "effect", "events/particles/flags", Old::Bit(0x1), "stay attached to marker"),
    row(HCE.0, HCE.1, "effect", "events/particles/flags", Old::Bit(0x2), "random initial angle"),
    row(HCE.0, HCE.1, "effect", "events/particles/flags", Old::Bit(0x4), "tint from object color"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x1), "velocity"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x2), "velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x4), "velocity cone angle"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x8), "angular velocity"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x10), "angular velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x20), "count"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x40), "count delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x80), "distribution radius"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x100), "distribution radius delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x200), "particle radius"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x400), "particle radius delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/A scales values", Old::Bit(0x800), "tint"),
    row(HCE.0, HCE.1, "effect", "events/parts/A scales values", Old::Bit(0x1), "velocity"),
    row(HCE.0, HCE.1, "effect", "events/parts/A scales values", Old::Bit(0x2), "velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/parts/A scales values", Old::Bit(0x4), "velocity cone angle"),
    row(HCE.0, HCE.1, "effect", "events/parts/A scales values", Old::Bit(0x8), "angular velocity"),
    row(HCE.0, HCE.1, "effect", "events/parts/A scales values", Old::Bit(0x10), "angular velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/parts/A scales values", Old::Bit(0x20), "type-specific scale"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x1), "velocity"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x2), "velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x4), "velocity cone angle"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x8), "angular velocity"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x10), "angular velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x20), "count"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x40), "count delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x80), "distribution radius"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x100), "distribution radius delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x200), "particle radius"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x400), "particle radius delta"),
    row(HCE.0, HCE.1, "effect", "events/particles/B scales values", Old::Bit(0x800), "tint"),
    row(HCE.0, HCE.1, "effect", "events/parts/B scales values", Old::Bit(0x1), "velocity"),
    row(HCE.0, HCE.1, "effect", "events/parts/B scales values", Old::Bit(0x2), "velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/parts/B scales values", Old::Bit(0x4), "velocity cone angle"),
    row(HCE.0, HCE.1, "effect", "events/parts/B scales values", Old::Bit(0x8), "angular velocity"),
    row(HCE.0, HCE.1, "effect", "events/parts/B scales values", Old::Bit(0x10), "angular velocity delta"),
    row(HCE.0, HCE.1, "effect", "events/parts/B scales values", Old::Bit(0x20), "type-specific scale"),
];

/// How a row's name reads in one tag's field.
enum Named {
    /// The option the row's number is, and whether it holds.
    Read(bool),
    /// The field's list has no such option.
    Missing,
    /// The name is another option, with this index: two options share it,
    /// or the row has the wrong name.
    Elsewhere(u32),
}

/// Every `(old, by name)` pair `path` reaches under `holder`.
fn read_both(holder: blam_tags::TagStruct<'_>, path: &str, row: &OptionRow, out: &mut Vec<(bool, Named)>) {
    // A field the struct does not hold is looked for in its first nested
    // struct, as an object's base fields are.
    let mut holder = holder;
    let first = path.split('/').next().unwrap_or(path);
    let first = first.split_once('#').map_or(first, |(name, _)| name);
    while !holder.fields().any(|field| field.clean_name() == first) {
        let Some(inner) = holder.fields_all().find_map(|field| field.as_struct()) else {
            return;
        };
        holder = inner;
    }
    match path.split_once('/') {
        // Each segment but the last is a block, walked element by element,
        // or a struct, walked into.
        Some((segment, rest)) => {
            let Some(field) = holder.field(segment) else {
                return;
            };
            if let Some(block) = field.as_block() {
                for element in block.iter() {
                    read_both(element, rest, row, out);
                }
            } else if let Some(inner) = field.as_struct() {
                read_both(inner, rest, row, out);
            }
        }
        None => {
            // A struct can hold several fields of one name (a shader's own
            // `flags` beside its kind's); the one with the option is read.
            // `name#n` is the nth field of that name.
            let (path, nth) = match path.split_once('#') {
                Some((name, n)) => (name, n.parse::<usize>().ok()),
                None => (path, None),
            };
            let named: Vec<_> = holder.fields().filter(|field| field.clean_name() == path).collect();
            let field = match nth {
                Some(n) => named.get(n).copied(),
                None => named.iter().find(|field| row.labels().any(|label| field.flag(label).is_some())).or(named.first()).copied(),
            };
            let Some(field) = field else {
                return;
            };
            let options = match field.options() {
                Some(blam_tags::TagOptions::Flags(options)) => options,
                _ => Vec::new(),
            };
            let old = match row.old {
                Old::Bit(bit) => {
                    let index = bit.trailing_zeros() as usize;
                    options.get(index).is_some_and(|option| option.is_set)
                }
                Old::Prefix(prefixes) => match field.value() {
                    Some(
                        blam_tags::TagFieldData::ByteFlags { names, .. }
                        | blam_tags::TagFieldData::WordFlags { names, .. }
                        | blam_tags::TagFieldData::LongFlags { names, .. },
                    ) => names.iter().any(|(_, name)| prefixes.iter().any(|prefix| name.starts_with(prefix))),
                    _ => return,
                },
                Old::Value(number) => match field.value().and_then(|value| match value {
                    blam_tags::TagFieldData::CharEnum { value, .. } => Some(i128::from(value)),
                    blam_tags::TagFieldData::ShortEnum { value, .. } => Some(i128::from(value)),
                    blam_tags::TagFieldData::LongEnum { value, .. } => Some(i128::from(value)),
                    _ => None,
                }) {
                    Some(value) => value == number,
                    None => return,
                },
            };
            let held_raw = match row.old {
                Old::Bit(bit) => match field.value() {
                    Some(blam_tags::TagFieldData::ByteFlags { value, .. }) => i128::from(value) & bit != 0,
                    Some(blam_tags::TagFieldData::WordFlags { value, .. }) => i128::from(value) & bit != 0,
                    Some(blam_tags::TagFieldData::LongFlags { value, .. }) => i128::from(value) & bit != 0,
                    _ => true,
                },
                Old::Value(_) => old,
                Old::Prefix(_) => false,
            };
            let label = row.labels().find(|label| field.flag(label).is_some());
            let found = label.and_then(|label| field.flag(label)).map(|option| option.bit() + u32::from(row.after));
            let set = |index: u32| options.get(index as usize).is_some_and(|option| option.is_set);
            let named = match found {
                None if matches!(row.old, Old::Prefix(_)) => Named::Read(false),
                // A layout with a shorter list stores nothing under the name;
                // only a tag that holds the bit or value has lost it.
                None if !held_raw => Named::Read(false),
                None => Named::Missing,
                Some(index) => {
                    let expected = match row.old {
                        Old::Bit(bit) => bit == 1 << index,
                        Old::Value(number) => number == i128::from(index),
                        Old::Prefix(_) => true,
                    };
                    match (expected, row.old) {
                        (false, _) => Named::Elsewhere(index),
                        (true, Old::Bit(_) | Old::Prefix(_)) => Named::Read(set(index)),
                        (true, Old::Value(_)) => Named::Read(label.is_some_and(|label| field.enum_is(label))),
                    }
                }
            };
            out.push((old, named));
        }
    }
}

/// G-opt: each option in [`OPTION_ROWS`] reads the same by name as by number
/// over every tag of its group in its kit, and its name is in the list of
/// every tag that holds its bit or value. Each control must disagree.
#[test]
fn gate_options() {
    use crate::core::source::read_tag_from_bytes;
    let definitions = crate::core::bundled::locate_definitions_root();
    let mut failures = Vec::new();
    let mut groups: Vec<(&str, GameId, &str)> = OPTION_ROWS.iter().map(|row| (row.kit, row.game, row.extension)).collect();
    groups.sort_by_key(|(kit, _, extension)| (*kit, *extension));
    groups.dedup();
    for (kit, game, extension) in groups {
        let Ok(root) = std::env::var(kit) else {
            eprintln!("skipping {game:?} {extension}: {kit} is not set");
            continue;
        };
        let rows: Vec<&OptionRow> =
            OPTION_ROWS.iter().filter(|row| row.kit == kit && row.extension == extension).collect();
        // Per row: tags, elements, disagreeing tags, tags missing the name,
        // and the other options the name resolves to.
        let mut tallies =
            vec![(0usize, 0usize, Vec::<String>::new(), Vec::<String>::new(), Vec::<u32>::new()); rows.len()];
        let suffix = format!(".{extension}");
        for entry in walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !path.to_string_lossy().ends_with(&suffix) {
                continue;
            }
            let Ok(bytes) = std::fs::read(path) else { continue };
            // A classic tag's layout follows its header's group, which Halo 2
            // stores reversed; MCC tags carry their own.
            let group = blam_tags::classic::ClassicHeader::parse(&bytes)
                .map_or(0, |(header, _)| u32::from_be_bytes(header.group_tag));
            let Ok(tag) = read_tag_from_bytes(&bytes, Some(game), Some(&definitions), group) else {
                continue;
            };
            let name = path.strip_prefix(&root).unwrap_or(path).display().to_string();
            for (row, tally) in rows.iter().zip(&mut tallies) {
                let mut read = Vec::new();
                read_both(tag.root(), row.path, row, &mut read);
                if read.is_empty() {
                    continue;
                }
                tally.0 += 1;
                tally.1 += read.len();
                for (old, named) in &read {
                    match named {
                        Named::Missing => tally.3.push(name.clone()),
                        Named::Elsewhere(index) if !tally.4.contains(index) => tally.4.push(*index),
                        Named::Elsewhere(_) => {}
                        Named::Read(named) if named != old => tally.2.push(name.clone()),
                        Named::Read(_) => {}
                    }
                }
                tally.2.dedup();
                tally.3.dedup();
            }
        }
        for (row, (tags, elements, disagree, missing, elsewhere)) in rows.iter().zip(&tallies) {
            eprintln!(
                "{game:?} {extension} {} {:?} {:?}: {tags} tags, {elements} reads, {} disagree, {} missing, elsewhere {elsewhere:?}",
                row.path, row.old, row.name, disagree.len(), missing.len()
            );
            let first = |list: &Vec<String>| list.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            if row.control {
                if disagree.is_empty() && elsewhere.is_empty() {
                    failures.push(format!("control {} {:?} never disagreed", row.name, row.old));
                }
            } else if !disagree.is_empty() || !missing.is_empty() || !elsewhere.is_empty() || *tags == 0 {
                failures.push(format!(
                    "{game:?} {extension} {} {:?} {:?}: {tags} tags; disagree {} ({}); missing {} ({}); resolves to {elsewhere:?}",
                    row.path, row.old, row.name, disagree.len(), first(disagree), missing.len(), first(missing)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn field_lines_name_each_top_level_field() {
    #[derive(Debug)]
    #[allow(dead_code, reason = "read through Debug")]
    struct Inner {
        deep: u8,
    }
    #[derive(Debug)]
    #[allow(dead_code, reason = "read through Debug")]
    struct Outer {
        first: Vec<u8>,
        quoted: &'static str,
        second: Inner,
    }
    let outer = |deep| Outer {
        first: vec![1, 2],
        quoted: "a \\\"}, fake: {",
        second: Inner { deep },
    };
    let mut out = String::new();
    field_lines(&mut out, "p", &outer(3));
    let names: Vec<_> = out
        .lines()
        .map(|row| row.split(' ').next().unwrap())
        .collect();
    assert_eq!(names, ["p.first", "p.quoted", "p.second"], "{out}");
    let mut changed = String::new();
    field_lines(&mut changed, "p", &outer(4));
    assert_eq!(
        out.lines().take(2).collect::<Vec<_>>(),
        changed.lines().take(2).collect::<Vec<_>>(),
        "untouched fields keep their lines"
    );
    assert_ne!(
        out.lines().nth(2),
        changed.lines().nth(2),
        "a nested change moves its field's line"
    );
    // A tuple struct or a bare map is one line under the prefix.
    #[derive(Debug)]
    struct Wrapped(
        #[allow(dead_code, reason = "read through Debug")] std::collections::BTreeMap<u8, u8>,
    );
    let wrapped = Wrapped([(1, 2)].into());
    let map = std::collections::BTreeMap::from([(3, 4)]);
    for value in [&wrapped as &dyn std::fmt::Debug, &map] {
        let mut out = String::new();
        field_lines(&mut out, "w", &value);
        assert_eq!(out.lines().count(), 1, "{out}");
        assert!(out.starts_with("w "), "{out}");
    }
}
