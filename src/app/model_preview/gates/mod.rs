//! Refactor gates: snapshots of what the preview produces, compared before and
//! after a change that should not alter it.
//!
//! `BABOON_GATE_DIR` names a folder outside the repo for the baselines;
//! `BABOON_GATE_BLESS=1` writes them instead of comparing. Each game's objects
//! load from its kit, named by the usual `BLAM_TEST_*` variable; a kit that is
//! not set is skipped and says so.
//!
//! - `load`: G-load, a line per field of what each object loads.
//! - `options`: G-opt, every option read the old way and by name.
//! - `pixels`: G-px, G-sim, G-gl and G-calls, from one set of shots.
//! - `field_lines`: the streaming `Debug` splitter the snapshots share.

use super::data::ModelPreviewData;
use super::harness::kit::kit_preview;
use crate::core::game::GameId;
use std::fmt::Write as _;
use std::path::PathBuf;

mod field_lines;
mod load;
mod options;
#[cfg(target_os = "macos")]
mod pixels;

pub(super) use field_lines::field_lines;
use field_lines::{debug_text, inline_field_lines, line};
pub(super) use load::LOAD_SET;

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
