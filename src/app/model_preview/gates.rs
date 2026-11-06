//! Refactor gates: snapshots of what the preview produces, compared before and
//! after a change that should not alter it.
//!
//! `BABOON_GATE_DIR` names a folder outside the repo for the baselines;
//! `BABOON_GATE_BLESS=1` writes them instead of comparing. Each game's objects
//! load from its kit, named by the usual `BLAM_TEST_*` variable; a kit that is
//! not set is skipped and says so.

use std::fmt::Write as _;
use std::path::PathBuf;

use super::loading::kit_object_tests::kit_preview;
use super::state::ModelPreviewData;
use crate::core::game::GameId;

/// The objects each game loads: a biped, weapons and vehicles, chosen to reach
/// as many of the preview's features as one load can.
const LOAD_SET: &[(&str, GameId, &[(&str, &str, &[u8; 4])])] = &[
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

/// A line per top-level field of `value`'s `Debug`.
pub(super) fn field_lines(out: &mut String, prefix: &str, value: &impl std::fmt::Debug) {
    let mut lines = FieldLines {
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
    };
    let _ = write!(lines, "{value:?}");
    lines.close();
}

/// What a load produced, without the ids that count loads.
fn load_snapshot(data: &ModelPreviewData) -> String {
    let mut out = String::new();
    line(&mut out, "render_model_path", &data.render_model_path);
    field_lines(&mut out, "preview", &data.preview);
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
        "h2_change_colors",
        &format!("{:?}", data.h2_change_colors),
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
        "h2_object_functions",
        &format!("{:?}", data.h2_object_functions),
    );
    line(&mut out, "h2_weapons", &format!("{:?}", data.h2_weapons));
    line(&mut out, "h2_attached", &format!("{:?}", data.h2_attached));
    line(
        &mut out,
        "scenario_bsps",
        &format!("{:?}", data.scenario_bsps),
    );
    line(&mut out, "notes", &format!("{:?}", data.notes));
    line(&mut out, "halo_ce", &format!("{:?}", data.halo_ce));
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
/// [`LOOKS`].
#[cfg(target_os = "macos")]
const PIXEL_SET: &[(&str, GameId, &[(&str, &str)])] = &[
    (
        "BLAM_TEST_HCEEK",
        GameId::HaloCe,
        &[
            ("characters\\cyborg\\cyborg", "biped"),
            ("vehicles\\warthog\\warthog", "vehicle"),
            ("weapons\\plasma rifle\\plasma rifle", "weapon"),
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

/// G-px, G-sim and G-gl: each object of [`PIXEL_SET`], in each of [`LOOKS`],
/// draws the same pixels as before, its slot keeps the same simulations (a
/// line per field, under `sim/`), and the paint leaves GL as it did (under
/// `gl/`). Blessing keeps every shot as a PNG beside the hashes; a shot that
/// differs is saved under `changed/` to set against it.
#[cfg(target_os = "macos")]
#[test]
fn gate_pixels() {
    use super::renderer::headless_shot::{Shot, ShotOptions, shoot};
    use crate::core::source::TagSource;
    let Some(dir) = gate_dir() else {
        eprintln!("skipping: BABOON_GATE_DIR is not set");
        return;
    };
    let (sim_dir, gl_dir, dir) = (dir.join("sim"), dir.join("gl"), dir.join("px"));
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
        let mut snapshot = String::new();
        let mut simulations = String::new();
        let mut gl_states = String::new();
        for (name, shown) in names.iter().zip(&images) {
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
}

const fn row(kit: &'static str, game: GameId, extension: &'static str, path: &'static str, old: Old, name: &'static str) -> OptionRow {
    OptionRow { kit, game, extension, path, old, name, control: false, after: false }
}

const fn after(kit: &'static str, game: GameId, extension: &'static str, path: &'static str, old: Old, name: &'static str) -> OptionRow {
    OptionRow { after: true, ..row(kit, game, extension, path, old, name) }
}

const HCE: (&str, GameId) = ("BLAM_TEST_HCEEK", GameId::HaloCe);
const H2: (&str, GameId) = ("BLAM_TEST_H2EK", GameId::Halo2);

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
    match path.split_once('/') {
        Some((block, rest)) => {
            let Some(block) = holder.field(block).and_then(|field| field.as_block()) else {
                return;
            };
            for element in block.iter() {
                read_both(element, rest, row, out);
            }
        }
        None => {
            let Some(field) = holder.field(path) else {
                return;
            };
            let Some(value) = holder.read_int_any(path) else {
                return;
            };
            let old = match row.old {
                Old::Bit(bit) => value & bit != 0,
                Old::Value(number) => value == number,
            };
            let found = field.flag(row.name).map(|option| option.bit() + u32::from(row.after));
            let set = |index: u32| match field.options() {
                Some(blam_tags::TagOptions::Flags(options)) => options.get(index as usize).is_some_and(|option| option.is_set),
                _ => false,
            };
            let named = match found {
                None => Named::Missing,
                Some(index) => {
                    let expected = match row.old {
                        Old::Bit(bit) => bit == 1 << index,
                        Old::Value(number) => number == i128::from(index),
                    };
                    match (expected, row.old) {
                        (false, _) => Named::Elsewhere(index),
                        (true, Old::Bit(_)) => Named::Read(set(index)),
                        (true, Old::Value(_)) => Named::Read(field.enum_is(row.name)),
                    }
                }
            };
            out.push((old, named));
        }
    }
}

/// G-opt: each option in [`OPTION_ROWS`] reads the same by name as by number
/// over every tag of its group in its kit, and its name is in every tag's
/// list. Each control must disagree.
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
