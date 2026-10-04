use super::*;

/// Containers mount in Unreal's order, the same one Chimp uses, so the two
/// agree on which mod's copy of a tag wins. Ordered by chunk number and
/// then case-sensitive path, `B_P` sorted before `a_P` and every plain `_P`
/// above `a_2_P`.
#[test]
fn containers_mount_in_unreals_order() {
    let paks = PathBuf::from("Paks");
    let utocs = [
        "~mods/z_P.utoc",
        "~mods/a_2_P.utoc",
        "pakchunk1-WinGDK.utoc",
        "~mods/B_P.utoc",
        "pakchunk0-WinGDK.utoc",
        "~mods/a_P.utoc",
    ]
    .iter()
    .map(|name| paks.join(name))
    .collect::<Vec<_>>();
    let ordered = mount_order(utocs)
        .iter()
        .map(|path| path.strip_prefix(&paks).unwrap().to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    assert_eq!(
        ordered,
        [
            "pakchunk0-WinGDK.utoc",
            "pakchunk1-WinGDK.utoc",
            "~mods/a_P.utoc",
            "~mods/B_P.utoc",
            "~mods/z_P.utoc",
            "~mods/a_2_P.utoc",
        ]
    );
}

/// A package is named by the engine's rule: a plugin's `Content/` mounts at
/// its own name, not `/game`, and maps are packages too.
#[test]
fn package_names_follow_the_mount_they_come_from() {
    assert_eq!(
        container_package_name("Meteorite/Content/Tags/sound/x-sound.uasset").as_deref(),
        Some("/game/tags/sound/x-sound"),
    );
    assert_eq!(
        container_package_name("Engine/Content/Maps/Probe.umap").as_deref(),
        Some("/engine/maps/probe"),
    );
    assert_eq!(
        container_package_name("Meteorite/Plugins/Tools/Content/UI/W_Hud.uasset").as_deref(),
        Some("/tools/ui/w_hud"),
    );
    assert_eq!(container_package_name("Meteorite/Content/Tags/x-sound.ubulk"), None);
}

static PAKS: std::sync::LazyLock<&'static str> =
    std::sync::LazyLock::new(|| crate::test_kits::leak(crate::test_kits::ce_paks()));

#[test]
fn container_ref_key_normalizes() {
    let skel = u32::from_be_bytes(*b"skel");
    // Backslashes → forward, uppercase → lower, trailing NUL stripped,
    // group FOURCC hex-prefixed — matching `build_container_set`'s logical key.
    assert_eq!(
        container_ref_key(skel, "Objects\\Characters\\Elite_AI\\Elite_AI\u{0}"),
        "736b656c:objects/characters/elite_ai/elite_ai"
    );
    // Idempotent on an already-normalized reference.
    assert_eq!(
        container_ref_key(skel, "objects/characters/elite_ai/elite_ai"),
        "736b656c:objects/characters/elite_ai/elite_ai"
    );
}

/// Mount the whole `Paks` directory through Baboon's set loader and read a
/// sample of tags via `read_entry`. Asserts scenarios (only in level chunks)
/// show up alongside pak0's shared tags. Skipped when the game isn't present.
#[test]
fn mount_container_set_and_read_tags() {
    if !Path::new(*PAKS).exists() {
        eprintln!("skipping: {} not present", *PAKS);
        return;
    }
    let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = TagNameIndex::load_from_definitions(&defs);
    let loaded = load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs)
        .expect("mount container set");

    assert!(
        loaded.entries.len() > 5000,
        "expected thousands of tags, got {}",
        loaded.entries.len()
    );
    assert_eq!(loaded.game.as_deref(), Some("haloce_evolved"));
    let TagSource::IoStoreContainerSet { ref containers, .. } = loaded.source else {
        panic!("expected a container set");
    };
    assert!(
        containers.len() > 10,
        "expected base + level chunks, got {}",
        containers.len()
    );

    // Scenarios live only in level chunks — proves multi-container merge.
    let scnr = u32::from_be_bytes(*b"scnr");
    let scenarios: Vec<&TagEntry> = loaded
        .entries
        .iter()
        .filter(|e| e.group_tag == scnr)
        .collect();
    eprintln!(
        "mounted {} packs, {} tags, {} scenarios",
        containers.len(),
        loaded.entries.len(),
        scenarios.len()
    );
    for s in scenarios.iter().take(3) {
        eprintln!("  scenario: {}", s.display_path);
    }
    assert!(
        scenarios.len() >= 10,
        "expected ~13 scenarios across level chunks, got {}",
        scenarios.len()
    );
    // Display paths are lowercased and Tags/-stripped.
    for s in &scenarios {
        assert!(
            s.display_path == s.display_path.to_ascii_lowercase(),
            "display path not lowercased: {}",
            s.display_path
        );
        assert!(!s.display_path.to_ascii_lowercase().contains("/tags/"));
    }

    // Read a sample (including every scenario) via the source-aware path.
    let mut sample: Vec<&TagEntry> = scenarios.clone();
    sample.extend(loaded.entries.iter().take(300));
    for entry in sample {
        let tag = read_entry(&loaded.source, entry)
            .unwrap_or_else(|e| panic!("read_entry failed for {}: {e}", entry.display_path));
        assert_eq!(
            tag.group().tag,
            entry.group_tag,
            "group mismatch for {}",
            entry.display_path
        );
    }

    // Reference resolution: a CE `.model` (hlmt) resolves its `animation`
    // (jmad) and `skeleton model` (skel) refs through the container index —
    // the payload the browser tree already mounts, by construction.
    let hlmt = u32::from_be_bytes(*b"hlmt");
    let mut checked = false;
    for entry in loaded.entries.iter().filter(|e| e.group_tag == hlmt) {
        let Ok(model) = read_entry(&loaded.source, entry) else {
            continue;
        };
        let root = model.root();
        let (Some((_, jmad_ref)), Some((_, skel_ref))) = (
            root.read_tag_ref_with_group("animation"),
            root.read_tag_ref_with_group("skeleton model"),
        ) else {
            continue;
        };
        if jmad_ref.trim().is_empty() || skel_ref.trim().is_empty() {
            continue;
        }
        let jmad = loaded
            .source
            .read_container_tag_by_ref(u32::from_be_bytes(*b"jmad"), &jmad_ref)
            .unwrap_or_else(|e| panic!("resolve jmad {jmad_ref}: {e}"));
        assert_eq!(jmad.group().tag, u32::from_be_bytes(*b"jmad"));
        let skel = loaded
            .source
            .read_container_tag_by_ref(u32::from_be_bytes(*b"skel"), &skel_ref)
            .unwrap_or_else(|e| panic!("resolve skel {skel_ref}: {e}"));
        assert_eq!(skel.group().tag, u32::from_be_bytes(*b"skel"));
        eprintln!(
            "resolved refs for {}: {jmad_ref} + {skel_ref}",
            entry.display_path
        );
        checked = true;
        break;
    }
    assert!(
        checked,
        "expected an hlmt carrying both animation and skeleton model refs"
    );
}
