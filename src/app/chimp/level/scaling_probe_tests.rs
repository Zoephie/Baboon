use super::*;

/// Split one cell's cost into the steps that make it up.
///
/// Reading a cell costs 329ms and the bytes arrive in under a millisecond,
/// so the time is in decoding — but decoding is a header parse, a class
/// lookup per export and a property decode per export, and which of those
/// it is decides whether anything can be done about it.
#[test]
#[ignore]
fn probe_cell_decode_breakdown() {
    use blam_tags::iostore::object::archive::ExportContext;
    use blam_tags::iostore::object::export::read_export_in;
    use blam_tags::iostore::package::builder::read_payloads;
    use blam_tags::iostore::package::zen::FZenPackageHeader;
    use std::io::Cursor;
    use std::time::Instant;

    let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
    let step: usize = std::env::var("BABOON_SCALE_STEP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let usmap = crate::app::chimp::load_chimp_usmap(None).expect("usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .step_by(step)
        .collect();

    let (mut read, mut parse, mut classes, mut decode) = (0.0, 0.0, 0.0, 0.0);
    let (mut exports_seen, mut per_cell) = (0usize, Vec::new());
    for cell in &cells {
        let cell_started = Instant::now();
        let started = Instant::now();
        let Ok(bytes) = world.read_package(cell) else {
            continue;
        };
        read += started.elapsed().as_secs_f64();

        let started = Instant::now();
        let Ok(header) = FZenPackageHeader::deserialize(
            &mut Cursor::new(&bytes),
            None,
            blam_tags::iostore::world::CE_TOC_VERSION,
            blam_tags::iostore::world::CE_HEADER_VERSION,
            None,
        ) else {
            continue;
        };
        let Ok(payloads) = read_payloads(&header, &bytes) else {
            continue;
        };
        let names = header.name_map.copy_raw_names();
        parse += started.elapsed().as_secs_f64();

        let started = Instant::now();
        let resolved: Vec<Option<String>> = header
            .export_map
            .iter()
            .map(|entry| world.class_key(&header, entry.class_index))
            .collect();
        classes += started.elapsed().as_secs_f64();

        let started = Instant::now();
        let resolver = world.resolver(&header, &bytes, &names);
        let bulk: Vec<(i64, i64)> = header
            .bulk_data
            .iter()
            .map(|entry| (entry.serial_offset, entry.serial_size))
            .collect();
        let context = ExportContext {
            bulk_data: &bulk,
            resolver: Some(&resolver),
        };
        for ((entry, payload), class) in header.export_map.iter().zip(&payloads).zip(&resolved)
        {
            exports_seen += 1;
            if let Some(class) = class.as_deref() {
                let _ = read_export_in(
                    payload,
                    &names,
                    world.usmap(),
                    class,
                    entry.object_flags,
                    &context,
                );
            }
        }
        decode += started.elapsed().as_secs_f64();
        per_cell.push(cell_started.elapsed().as_secs_f64());
    }

    per_cell.sort_by(f64::total_cmp);
    let total: f64 = per_cell.iter().sum();
    let at = |q: f64| per_cell[((per_cell.len() as f64 - 1.0) * q) as usize];
    eprintln!(
        "{} cells, {exports_seen} exports, {total:.1}s\n\
             \x20 read bytes    {read:6.1}s\n\
             \x20 parse header  {parse:6.1}s\n\
             \x20 resolve class {classes:6.1}s\n\
             \x20 decode props  {decode:6.1}s\n\
             \x20 per cell: median {:.0}ms  p90 {:.0}ms  p99 {:.0}ms  max {:.0}ms",
        cells.len(),
        at(0.5) * 1000.0,
        at(0.9) * 1000.0,
        at(0.99) * 1000.0,
        per_cell.last().copied().unwrap_or(0.0) * 1000.0,
    );
}

/// Find out what a cell read actually costs, and whether threads help.
///
/// Parallelising the walk gained 13% on sixteen threads, which is not what
/// independent work does — so this splits the cost into decompressing the
/// package and decoding its exports, and times the walk at several thread
/// counts, rather than guessing which one is the wall.
#[test]
#[ignore]
fn probe_cell_read_scaling() {
    let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
    let step: usize = std::env::var("BABOON_SCALE_STEP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let usmap = crate::app::chimp::load_chimp_usmap(None).expect("usmap");
    let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
    let cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
        .map(|record| record.name.clone())
        .step_by(step)
        .collect();
    eprintln!("{} cells", cells.len());

    let started = std::time::Instant::now();
    let mut bytes = 0usize;
    for cell in &cells {
        if let Ok(data) = world.read_package(cell) {
            bytes += data.len();
        }
    }
    let read = started.elapsed().as_secs_f64();
    eprintln!(
        "  read_package only:      {read:.1}s  ({:.1} MiB, {:.1} MiB/s)",
        bytes as f64 / (1024.0 * 1024.0),
        bytes as f64 / (1024.0 * 1024.0) / read
    );

    let started = std::time::Instant::now();
    for cell in &cells {
        let _ = crate::app::chimp::load_chimp_document(&world, cell);
    }
    eprintln!(
        "  load_chimp_document:    {:.1}s",
        started.elapsed().as_secs_f64()
    );

    for threads in [1usize, 2, 4, 8, 16] {
        let started = std::time::Instant::now();
        let scene = read_cells(&world, &cells, threads, &|_| {});
        eprintln!(
            "  read_cells x{threads:<2}          {:.1}s  ({} placements)",
            started.elapsed().as_secs_f64(),
            scene.placements.len()
        );
    }
}
