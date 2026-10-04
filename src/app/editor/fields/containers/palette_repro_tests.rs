use super::*;

#[test]
fn block_jump_matches_exact_paths_and_opens_ancestors() {
    let target = "regions#4[2]/permutations#7";
    assert!(block_paths_match(target, "regions[2]/permutations"));
    assert!(!block_paths_match(target, "regions[1]/permutations"));
    assert!(block_jump_opens_path(target, "regions#4"));
    assert!(block_jump_opens_path(target, target));
    assert!(!block_jump_opens_path(target, "materials#5"));
}

#[test]
fn block_jump_scroll_keeps_two_rows_of_leading_context() {
    let header = egui::Rect::from_min_size(
        egui::pos2(12.0, 240.0),
        egui::vec2(600.0, 40.0),
    );
    let target = block_jump_scroll_rect(header);

    assert_eq!(target.top(), 160.0);
    assert_eq!(target.bottom(), header.bottom());
    assert_eq!(target.left(), header.left());
    assert_eq!(target.right(), header.right());
}

#[test]
fn block_index_target_uses_the_renderers_exact_widget_path() {
    let tag = TagFile::new(crate::app::test_definition_path(
        "haloreach_mcc/test_tag.json",
    ))
    .unwrap();
    let root = tag.root();
    let index = root
        .fields_all()
        .find(|field| field.name() == "short block index")
        .unwrap();
    let target = block_index_target_options(&root, &index, Some(root), "")
        .unwrap()
        .path;

    assert!(target.contains('#'), "target should carry an exact ordinal");
    assert!(
        root.field_path(&target).is_some(),
        "target path should resolve exactly"
    );
}

/// Reproduction probe for "a tag added to the scenario vehicle palette does
/// not appear in the vehicles block's dropdown until save + reopen".
/// Runs the same add-then-set-reference flow on every editing kit present.
#[test]
fn palette_addition_reaches_the_block_index_dropdown() {
    let cases = [
        ("halo3_mcc", "levels/multi/riverworld/riverworld.scenario"),
        ("haloreach_mcc", "levels/multi/35_island/35_island.scenario"),
        (
            "halo2_mcc",
            "scenarios/solo/05a_deltaapproach/05a_deltaapproach.scenario",
        ),
        ("haloce_mcc", "levels/d40/d40.scenario"),
    ];
    let defs = std::path::Path::new("definitions");
    let names = crate::core::format::TagNameIndex::default();
    let group = u32::from_be_bytes(*b"scnr");
    let mut failures = Vec::new();

    for (game, rel) in cases {
        let tag_path = std::path::Path::new(crate::test_kits::tag_path(game, "")).join(rel);
        if !tag_path.exists() {
            eprintln!("skip {game}: {} missing", tag_path.display());
            continue;
        }
        let Ok(mut tag) =
            crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
        else {
            eprintln!("skip {game}: could not read scenario");
            continue;
        };

        // The dropdown a `vehicles` element's block-index field would show.
        let options = |tag: &blam_tags::TagFile| -> Option<Vec<String>> {
            let root = tag.root();
            for field in root.fields_all() {
                if let Some(block) = field.as_block()
                    && field.name() == "vehicles"
                    && let Some(element) = block.element(0)
                {
                    for sub in element.fields_all() {
                        if sub.definition().block_index_target().is_some()
                            && let Some(target) = block_index_target_options(
                                &element,
                                &sub,
                                Some(root),
                                "vehicles[0]",
                            )
                            && target.path.contains("palette")
                        {
                            return Some(block_index_target_labels(
                                Some(root),
                                &target,
                                &names,
                            ));
                        }
                    }
                }
            }
            None
        };
        let palette_len = |tag: &blam_tags::TagFile| -> Option<usize> {
            tag.root().fields_all().find_map(|field| {
                (field.name() == "vehicle palette")
                    .then(|| field.as_block().map(|block| block.len()))
                    .flatten()
            })
        };

        let Some(before) = options(&tag) else {
            eprintln!("skip {game}: no vehicles block-index field resolved");
            continue;
        };
        let before_len = palette_len(&tag).unwrap_or(0);
        let mut dirty = Dirty::default();
        let status = crate::core::document::apply::apply_block_ops(
            &mut tag,
            vec![BlockOp {
                path: "vehicle palette".to_owned(),
                kind: BlockOpKind::Add,
            }],
            &mut dirty,
        );
        let after_len = palette_len(&tag).unwrap_or(0);
        let after = options(&tag).unwrap_or_default();
        eprintln!(
            "{game}: palette {before_len} -> {after_len}, dropdown {} -> {} ({status:?})",
            before.len(),
            after.len()
        );
        if after_len != before_len + 1 {
            failures.push(format!("{game}: palette did not grow"));
            continue;
        }
        if after.len() != before.len() + 1 {
            failures.push(format!(
                "{game}: dropdown stayed at {} option(s) after the palette grew to {after_len}",
                after.len()
            ));
            continue;
        }
        // And the label follows the reference the user then sets.
        let edit = PendingFieldEdit {
            path: format!("vehicle palette[{before_len}]/name"),
            input: "objects/vehicles/warthog/warthog.vehicle".to_owned(),
        };
        let applied =
            crate::core::document::apply::apply_pending_edits(&mut tag, vec![edit], &mut dirty);
        let labelled = options(&tag).unwrap_or_default();
        let last = labelled.last().cloned().unwrap_or_default();
        eprintln!("{game}: new label {last:?} ({:?})", applied.status);
        if !last.contains("warthog") {
            failures.push(format!(
                "{game}: label did not pick up the reference: {last:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Every block-index field in a tag, at every depth, with the struct path
/// the app itself would render it under (`name#ordinal` segments) -- so the
/// ancestor walk in `block_index_target_options` is exercised through
/// `root.descend`, not just the root-level shortcut.
fn collect_block_index_fields(
    st: &blam_tags::TagStruct<'_>,
    path: &str,
    out: &mut Vec<String>,
    depth: usize,
) {
    if depth > 6 || out.len() > 400 {
        return;
    }
    for field in st.fields_all() {
        let field_path = crate::core::document::value::append_field_path_for(path, &field);
        if field.definition().block_index_target().is_some() {
            out.push(path.to_owned());
        }
        if let Some(block) = field.as_block()
            && block.len() > 0
            && let Some(element) = block.element(0)
        {
            let element_path = format!("{field_path}[0]");
            collect_block_index_fields(&element, &element_path, out, depth + 1);
        }
    }
}

/// The general form of Crisp's report: for a block-index field anywhere in
/// a tag, adding an element to the block it targets must be offered by the
/// dropdown immediately, with no save and reopen.
#[test]
fn adding_to_a_targeted_block_reaches_its_dropdown_at_every_depth() {
    let defs = std::path::Path::new("definitions");
    let cases = [
        (
            "halo3_mcc",
            "levels/multi/riverworld/riverworld.scenario",
            *b"scnr",
        ),
        (
            "haloreach_mcc",
            "levels/multi/35_island/35_island.scenario",
            *b"scnr",
        ),
    ];
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for (game, rel, group_bytes) in cases {
        let tag_path = std::path::Path::new(crate::test_kits::tag_path(game, "")).join(rel);
        if !tag_path.exists() {
            eprintln!("skip {game}: {} missing", tag_path.display());
            continue;
        }
        let group = u32::from_be_bytes(group_bytes);
        let Ok(tag) = crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
        else {
            eprintln!("skip {game}: unreadable");
            continue;
        };
        let mut struct_paths = Vec::new();
        collect_block_index_fields(&tag.root(), "", &mut struct_paths, 0);
        struct_paths.sort();
        struct_paths.dedup();
        eprintln!(
            "{game}: {} struct(s) holding block-index fields",
            struct_paths.len()
        );

        for struct_path in struct_paths.iter().take(40) {
            // Re-read per case: each one mutates the tag.
            let Ok(mut tag) =
                crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
            else {
                continue;
            };
            let resolve = |tag: &blam_tags::TagFile| -> Option<(String, usize)> {
                let root = tag.root();
                let st = if struct_path.is_empty() {
                    root
                } else {
                    root.descend(struct_path)?
                };
                for field in st.fields_all() {
                    if field.definition().block_index_target().is_some()
                        && let Some(target) =
                            block_index_target_options(&st, &field, Some(root), struct_path)
                    {
                        return Some((target.path, target.len));
                    }
                }
                None
            };
            let Some((target, before)) = resolve(&tag) else {
                continue;
            };
            let mut dirty = Dirty::default();
            let status = crate::core::document::apply::apply_block_ops(
                &mut tag,
                vec![BlockOp {
                    path: target.clone(),
                    kind: BlockOpKind::Add,
                }],
                &mut dirty,
            );
            if status.as_deref().is_some_and(|s| s.contains("failed")) {
                eprintln!("  {struct_path:?} -> {target:?}: add failed: {status:?}");
                continue;
            }
            let after = resolve(&tag).map(|(_, n)| n).unwrap_or(0);
            checked += 1;
            if after != before + 1 {
                failures.push(format!(
                    "{game} {struct_path:?} -> target {target:?}: {before} option(s) before, \
                         {after} after adding an element (expected {})",
                    before + 1
                ));
            }
        }
    }
    eprintln!("checked {checked} block-index field(s)");
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Crisp's report, stated exactly: adding an element to a block must make
/// that block's own instance selector offer it, without a save and reopen.
/// The selector lists `0..block.len()`, so this checks the length the
/// renderer would read on the next frame.
#[test]
fn adding_an_element_grows_the_blocks_own_length() {
    let defs = std::path::Path::new("definitions");
    let cases = [
        (
            "halo3_mcc",
            "levels/multi/riverworld/riverworld.scenario",
            *b"scnr",
        ),
        (
            "haloreach_mcc",
            "levels/multi/35_island/35_island.scenario",
            *b"scnr",
        ),
        (
            "halo2_mcc",
            "scenarios/solo/05a_deltaapproach/05a_deltaapproach.scenario",
            *b"scnr",
        ),
        ("haloce_mcc", "levels/d40/d40.scenario", *b"scnr"),
    ];
    let mut failures = Vec::new();

    for (game, rel, group_bytes) in cases {
        let tag_path = std::path::Path::new(crate::test_kits::tag_path(game, "")).join(rel);
        if !tag_path.exists() {
            eprintln!("skip {game}: missing");
            continue;
        }
        let group = u32::from_be_bytes(group_bytes);
        let Ok(tag) = crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
        else {
            eprintln!("skip {game}: unreadable");
            continue;
        };
        // Root-level blocks, split by whether they start empty: an
        // empty-on-disk block is the case that has bitten before.
        let mut blocks: Vec<(String, usize)> = Vec::new();
        for field in tag.root().fields_all() {
            if let Some(block) = field.as_block() {
                blocks.push((field.name().to_owned(), block.len()));
            }
        }
        let empty = blocks.iter().filter(|(_, n)| *n == 0).count();
        eprintln!("{game}: {} root block(s), {empty} empty", blocks.len());

        let mut checked = 0usize;
        let mut stale = 0usize;
        for (name, before) in blocks {
            let Ok(mut tag) =
                crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
            else {
                continue;
            };
            let mut dirty = Dirty::default();
            let status = crate::core::document::apply::apply_block_ops(
                &mut tag,
                vec![BlockOp {
                    path: name.clone(),
                    kind: BlockOpKind::Add,
                }],
                &mut dirty,
            );
            if status.as_deref().is_some_and(|s| s.contains("failed")) {
                continue;
            }
            let after = tag
                .root()
                .fields_all()
                .find_map(|field| {
                    (field.name() == name)
                        .then(|| field.as_block().map(|block| block.len()))
                        .flatten()
                })
                .unwrap_or(0);
            checked += 1;
            if after != before + 1 {
                stale += 1;
                failures.push(format!(
                    "{game} block {name:?}: len {before} -> {after} after adding an element \
                         (status {status:?})"
                ));
            }
        }
        eprintln!("{game}: checked {checked} block(s), {stale} stale");
    }
    assert!(
        failures.is_empty(),
        "{} failure(s):\n{failures:#?}",
        failures.len()
    );
}
