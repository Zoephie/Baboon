use super::*;

/// Campaign Evolved's cook puts a level's generated tags under
/// `_Generated_`, but the references baked into the tag data still use the
/// pre-cook path — `c10.scenario` points at `levels\halo1\solo\c10\level_a`
/// while the payload mounts at `.../c10/_generated_/level_a`. Without the
/// fallback, exporting a CE scenario's geometry resolved none of its \\Ps.
#[test]
fn container_lookup_falls_back_through_generated_folder() {
    let sbsp = u32::from_be_bytes(*b"sbsp");
    let mut index = ContainerTagIndex::default();
    index.insert(
        container_ref_key(sbsp, "levels/halo1/solo/c10/_generated_/level_a"),
        3,
        "Meteorite/Content/Tags/Levels/Halo1/Solo/C10/_Generated_/level_a-scenario_structure_bsp.ubulk"
            .to_owned(),
    );

    // The reference as the scenario stores it: backslashes, no `_Generated_`.
    let (container, rel) = index
        .lookup(sbsp, "levels\\halo1\\solo\\c10\\level_a")
        .expect("reference should resolve through the _Generated_ folder");
    assert_eq!(container, 3);
    assert!(rel.ends_with("level_a-scenario_structure_bsp.ubulk"));

    // The literal mounted path still resolves.
    assert!(
        index
            .lookup(sbsp, "levels\\halo1\\solo\\c10\\_generated_\\level_a")
            .is_some()
    );
    // Wrong group and unknown paths still miss.
    assert!(
        index
            .lookup(
                u32::from_be_bytes(*b"scnr"),
                "levels\\halo1\\solo\\c10\\level_a"
            )
            .is_none()
    );
    assert!(
        index
            .lookup(sbsp, "levels\\halo1\\solo\\c99\\nope")
            .is_none()
    );
}

/// An exact hit must never be shadowed by a `_Generated_` neighbour.
#[test]
fn exact_container_match_wins_over_generated_fallback() {
    let sbsp = u32::from_be_bytes(*b"sbsp");
    let mut index = ContainerTagIndex::default();
    index.insert(
        container_ref_key(sbsp, "levels/x/bsp"),
        1,
        "exact.ubulk".to_owned(),
    );
    index.insert(
        container_ref_key(sbsp, "levels/x/_generated_/bsp"),
        2,
        "generated.ubulk".to_owned(),
    );
    assert_eq!(
        index.lookup(sbsp, "levels\\x\\bsp"),
        Some((1, "exact.ubulk"))
    );
}
