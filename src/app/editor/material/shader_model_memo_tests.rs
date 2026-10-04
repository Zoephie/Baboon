use super::*;

/// The shader grid's model is built once for a revision of the document,
/// not on every frame, and again when the document changes.
#[test]
fn the_shader_grid_is_built_once_per_revision() {
    let root = crate::test_kits::h3ek_tags();
    if !root.is_dir() {
        eprintln!("skipping: {} not present", root.display());
        return;
    }
    let definitions_root = crate::core::bundled::locate_definitions_root();
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: Some(GameId::Halo3),
        definitions_root: definitions_root.clone(),
    };
    let names = TagNameIndex::default();
    let mut rmdf_cache = HashMap::new();
    let mut rmop_cache = HashMap::new();
    let mut h2_templates = H2TemplateCache::default();
    // A shader whose grid actually builds, so the path measured is the
    // H3+ one rather than the raw-field fallback.
    let (tag, entry) = walkdir::WalkDir::new(root.join("shaders"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|item| item.path().extension().is_some_and(|ext| ext == "shader"))
        .find_map(|item| {
            let entry = crate::core::source::loose_file_entry(&root, item.path(), &names).ok()??;
            let tag = crate::core::source::read_tag_at_path(
                item.path(),
                Some(GameId::Halo3),
                Some(&definitions_root),
                entry.group_tag,
            )
            .ok()?;
            build_shader_editor_model(
                &tag,
                entry.group_tag,
                Some(&source),
                &mut rmdf_cache,
                &mut rmop_cache,
            )?;
            Some((tag, entry))
        })
        .expect("a Halo 3 shader whose grid builds");

    let ctx = egui::Context::default();
    let mut draw = |revision: (u64, u64, u64, u64)| {
        let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                crate::app::editor::fields::extracted_tests::tests::with_test_edit_context(
                    |edit| {
                        draw_material_tag(
                            ui,
                            &tag,
                            revision,
                            &entry,
                            &names,
                            Some(&source),
                            &mut rmdf_cache,
                            &mut rmop_cache,
                            &mut h2_templates,
                            &mut None,
                            &mut None,
                            false,
                            edit,
                        );
                    },
                );
            });
        });
    };
    SHADER_MODELS_BUILT.with(|built| built.set(0));
    for _ in 0..3 {
        draw((7, 1, 0, 0));
    }
    assert_eq!(SHADER_MODELS_BUILT.with(std::cell::Cell::get), 1);
    let memo = ctx.data(|data| {
        data.get_temp::<((u64, u64, u64, u64), Option<Arc<ShaderEditorModel>>)>(egui::Id::new((
            "shader_editor_model",
            7u64,
        )))
    });
    assert!(
        memo.is_some_and(|(_, model)| model.is_some()),
        "the grid was drawn"
    );
    draw((7, 2, 0, 0));
    assert_eq!(
        SHADER_MODELS_BUILT.with(std::cell::Cell::get),
        2,
        "an edit rebuilds it"
    );
    draw((7, 2, 0, 1));
    assert_eq!(
        SHADER_MODELS_BUILT.with(std::cell::Cell::get),
        3,
        "and so does a saved definition or option"
    );
}
