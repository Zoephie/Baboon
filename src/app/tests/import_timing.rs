//! Where the wall-clock of one conversion actually goes.
//!
//! Diagnostic, not a gate: run with `--ignored --nocapture` against real kits.
//! Every number here is measured on this machine's installed kits, so it is a
//! profile rather than an assertion.

use crate::app::*;
use std::time::Instant;

fn kit(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from("D:/SteamLibrary/steamapps/common")
        .join(name)
        .join("tags");
    path.is_dir().then_some(path)
}

/// First `.bitmap` under `root`, depth-first by sorted name.
fn first_bitmap(root: &Path) -> Option<PathBuf> {
    let mut found: Option<PathBuf> = None;
    for path in blam_tags::convert::walk_files(root) {
        if path.extension().and_then(|e| e.to_str()) == Some("bitmap")
            && found.as_ref().is_none_or(|best| &path < best)
        {
            found = Some(path);
        }
    }
    found
}

#[test]
#[ignore = "diagnostic; needs H3EK and HREK"]
fn report_where_a_single_bitmap_conversion_spends_its_time() {
    let (Some(source_tags), Some(target_tags)) = (kit("H3EK"), kit("HREK")) else {
        eprintln!("skipping: needs H3EK and HREK under D:/SteamLibrary");
        return;
    };
    let definitions = locate_definitions_root();

    let clock = Instant::now();
    let Some(source_path) = first_bitmap(&source_tags) else {
        eprintln!("skipping: H3EK ships no .bitmap");
        return;
    };
    eprintln!(
        "find a source bitmap:            {:>8.0} ms  ({})",
        clock.elapsed().as_secs_f64() * 1000.0,
        source_path.display()
    );

    let clock = Instant::now();
    let source = crate::source::read_tag_at_path(
        &source_path,
        Some("halo3_mcc"),
        Some(&definitions),
        u32::from_be_bytes(*b"bitm"),
    )
    .unwrap();
    eprintln!(
        "read the source tag:             {:>8.0} ms  ({} bytes on disk)",
        clock.elapsed().as_secs_f64() * 1000.0,
        fs::metadata(&source_path).map(|m| m.len()).unwrap_or(0)
    );

    let clock = Instant::now();
    let target_groups = blam_tags::convert::GameTagIndex::load(&definitions, "haloreach_mcc").unwrap();
    eprintln!(
        "GameTagIndex::load:              {:>8.0} ms",
        clock.elapsed().as_secs_f64() * 1000.0
    );

    let clock = Instant::now();
    let templates = blam_tags::convert::NativeTemplateIndex::build(&target_tags, &target_groups);
    eprintln!(
        "NativeTemplateIndex::build:      {:>8.0} ms  (walks the whole target kit)",
        clock.elapsed().as_secs_f64() * 1000.0
    );

    // With templates: this is what the dialog does today.
    let clock = Instant::now();
    let with = blam_tags::convert::analyze_conversion_with_templates(
        &source,
        "halo3_mcc",
        "haloreach_mcc",
        &definitions,
        Some(&templates),
    );
    let with_ms = clock.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "analyze WITH templates:          {:>8.0} ms  -> {}",
        with_ms,
        match &with {
            Ok(draft) => format!(
                "ok, template {}",
                draft
                    .native_layout_template
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(none found)".to_owned())
            ),
            Err(error) => format!("ERR {error}"),
        }
    );

    // Second call on the same index: the byte cache should make it ~free, which
    // tells us whether the cost is finding the template or converting the tag.
    let clock = Instant::now();
    let _ = blam_tags::convert::analyze_conversion_with_templates(
        &source,
        "halo3_mcc",
        "haloreach_mcc",
        &definitions,
        Some(&templates),
    );
    eprintln!(
        "analyze again (template cached): {:>8.0} ms",
        clock.elapsed().as_secs_f64() * 1000.0
    );

    // Without templates: isolates the conversion itself from template hunting.
    let clock = Instant::now();
    let _ = blam_tags::convert::analyze_conversion_with_templates(
        &source,
        "halo3_mcc",
        "haloreach_mcc",
        &definitions,
        None,
    );
    eprintln!(
        "analyze WITHOUT templates:       {:>8.0} ms",
        clock.elapsed().as_secs_f64() * 1000.0
    );
}

/// Why the hunt scans everything: what the acceptance test actually accepts.
///
/// `find_native_target_template` walks a group's tags in sorted order and takes
/// the first whose header `version` is not `u32::MAX`. Stock tags are written
/// with `-1` when no per-file source revision is known — so the question this
/// answers is how many tags in a real kit have one at all, and how deep into the
/// sorted list the first of them sits.
#[test]
#[ignore = "diagnostic; needs HREK"]
fn report_how_deep_the_template_hunt_has_to_dig() {
    let Some(target_tags) = kit("HREK") else {
        eprintln!("skipping: needs HREK");
        return;
    };
    let all = blam_tags::convert::walk_files(&target_tags);
    for extension in ["bitmap", "weapon", "particle", "model", "biped", "effect"] {
        let mut paths: Vec<PathBuf> = all
            .iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(extension))
            .cloned()
            .collect();
        paths.sort();
        if paths.is_empty() {
            eprintln!(".{extension:<10} none shipped");
            continue;
        }
        let clock = Instant::now();
        let mut accepted = 0usize;
        let mut first_accepted = None;
        let mut bytes_to_first = 0u64;
        let mut total_bytes = 0u64;
        for (index, path) in paths.iter().enumerate() {
            let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            total_bytes += size;
            if first_accepted.is_none() {
                bytes_to_first += size;
            }
            let Ok(tag) = TagFile::read(path) else { continue };
            if tag.header.version != u32::MAX {
                accepted += 1;
                if first_accepted.is_none() {
                    first_accepted = Some(index);
                }
            }
        }
        eprintln!(
            ".{extension:<10} {:>5} shipped ({:>7.1} MB), {accepted:>5} accepted; first at              index {:?} after {:.1} MB; full scan {:.0} ms",
            paths.len(),
            total_bytes as f64 / 1_048_576.0,
            first_accepted,
            bytes_to_first as f64 / 1_048_576.0,
            clock.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Would sorting candidates by size find a usable template sooner?
///
/// A template is wanted for its embedded *layout*, not its content, so the
/// smallest tag of a group is as good as the largest and costs a fraction to
/// read. If the accepted set is non-empty, this says how much the cheap-first
/// order would have saved.
#[test]
#[ignore = "diagnostic; needs HREK"]
fn report_whether_smallest_first_finds_a_template_sooner() {
    let Some(target_tags) = kit("HREK") else {
        eprintln!("skipping: needs HREK");
        return;
    };
    let all = blam_tags::convert::walk_files(&target_tags);
    for extension in ["bitmap", "weapon", "particle", "model", "biped", "effect"] {
        let mut sized: Vec<(u64, PathBuf)> = all
            .iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(extension))
            .map(|p| (fs::metadata(p).map(|m| m.len()).unwrap_or(0), p.clone()))
            .collect();
        if sized.is_empty() {
            continue;
        }
        sized.sort();
        let clock = Instant::now();
        let mut read_bytes = 0u64;
        let mut opened = 0usize;
        let mut found = None;
        for (size, path) in &sized {
            opened += 1;
            read_bytes += size;
            let Ok(tag) = TagFile::read(path) else { continue };
            if tag.header.version != u32::MAX {
                found = Some(path.clone());
                break;
            }
        }
        eprintln!(
            ".{extension:<10} smallest-first: {opened} read(s), {:.1} MB, {:.0} ms -> {}",
            read_bytes as f64 / 1_048_576.0,
            clock.elapsed().as_secs_f64() * 1000.0,
            found
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "nothing accepted".to_owned())
        );
    }
}
