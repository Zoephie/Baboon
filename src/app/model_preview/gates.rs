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
/// found by bracket depth, outside quoted text.
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
}

impl FieldLines<'_> {
    fn finish_field(&mut self) {
        if let Some((name, length, hash)) = self.field.take() {
            let _ = writeln!(self.out, "{}.{name} {length} {hash:016x}", self.prefix);
        }
    }

    fn step(&mut self, byte: u8) {
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
        match byte {
            b'"' => self.quoted = true,
            b'{' | b'[' | b'(' => {
                self.depth += 1;
                if self.depth == 1 {
                    self.naming = Some(String::new());
                }
            }
            b'}' | b']' | b')' => self.depth = self.depth.saturating_sub(1),
            b',' if self.depth == 1 => self.naming = Some(String::new()),
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
fn field_lines(out: &mut String, prefix: &str, value: &impl std::fmt::Debug) {
    let mut lines = FieldLines {
        out,
        prefix,
        field: None,
        depth: 0,
        quoted: false,
        escaped: false,
        naming: None,
    };
    let _ = write!(lines, "{value:?}");
    lines.finish_field();
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
            let _ = writeln!(snapshot, "== {path}.{extension}");
            snapshot.push_str(&load_snapshot(&data));
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

#[test]
fn field_lines_name_each_top_level_field() {
    #[derive(Debug)]
    struct Inner {
        deep: u8,
    }
    #[derive(Debug)]
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
}
