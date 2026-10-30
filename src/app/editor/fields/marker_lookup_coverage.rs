//! Saved-tag evidence for the marker fields covered by the installed-kit audit.
use super::*;
use blam_tags::string_editor::TagSource;

#[derive(serde::Deserialize)]
struct Case {
    engine: String,
    group: String,
    r#struct: String,
    field: String,
    tag: String,
    #[serde(rename = "field_path")]
    path: String,
    value: String,
}

struct Source {
    root: PathBuf,
    definitions: PathBuf,
    game: GameId,
    names: serde_json::Value,
    tags: RefCell<HashMap<PathBuf, Arc<blam_tags::TagFile>>>,
}

impl TagSource for Source {
    fn tag(&self, group: u32, path: &str) -> Option<Arc<blam_tags::TagFile>> {
        let key = String::from_utf8(group.to_be_bytes().to_vec()).ok()?;
        let extension = self.names["tag_index"][key].as_str()?;
        let path = self.root.join(path.replace('\\', "/")).with_extension(extension);
        if let Some(tag) = self.tags.borrow().get(&path) { return Some(tag.clone()); }
        let tag = Arc::new(crate::core::source::read_tag_at_path(
            &path, Some(self.game), Some(&self.definitions), group).ok()?);
        self.tags.borrow_mut().insert(path, tag.clone());
        Some(tag)
    }
}

#[test]
fn audited_marker_fields_resolve_markers_in_saved_kits() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("marker_lookup_cases.json")).unwrap();
    let definitions = locate_definitions_root();
    let mut exercised = 0;
    for engine in ["halo2_mcc", "halo3_mcc", "halo3odst_mcc", "haloreach_mcc", "halo2amp_mcc"] {
        let root = PathBuf::from(crate::core::test_kits::tag_path(engine, ""));
        if !root.exists() { eprintln!("skip {engine}: kit not available"); continue; }
        let game = GameId::from_id(engine).unwrap();
        let names: serde_json::Value = serde_json::from_slice(&std::fs::read(definitions.join(engine).join("_meta.json")).unwrap()).unwrap();
        let source = Source { root, definitions: definitions.clone(), game, names, tags: Default::default() };
        let editors = StringEditors::load(definitions.join(engine)).unwrap();
        let mut count = 0;
        for case in cases.iter().filter(|case| case.engine == engine) {
            let group = source.names["tag_index"].as_object().unwrap().iter()
                .find(|(_, value)| value.as_str() == Some(&case.group))
                .map(|(key, _)| u32::from_be_bytes(key.as_bytes().try_into().unwrap())).unwrap();
            let tag_path = Path::new(&case.tag).with_extension("").to_string_lossy().into_owned();
            let tag = source.tag(group, &tag_path).unwrap_or_else(|| panic!("{engine}: {}", case.tag));
            assert_eq!(editors.editor_at(tag.root(), &case.path), Some("marker_names"),
                "{engine}: {} {} ({}/{})", case.tag, case.path, case.r#struct, case.field);
            let menu = editors.menu("marker_names", &EditorContext {
                tag: &tag, tag_path: &tag_path, field_path: &case.path,
            }, &source);
            assert!(menu.picks().iter().any(|value| value.eq_ignore_ascii_case(&case.value)),
                "{engine}: {} {} should offer {:?}, note={:?}", case.tag, case.path, case.value, menu.note);
            assert!(menu.accepts(""));
            assert!(menu.accepts("runtime_or_custom_marker"), "marker menus remain suggestions");
            count += 1;
        }
        eprintln!("{engine}: {count} saved marker field contexts checked");
        exercised += count;
    }
    if ["BLAM_TEST_H2EK", "BLAM_TEST_H3EK", "BLAM_TEST_ODSTEK", "BLAM_TEST_HREK", "BLAM_TEST_H2AEK"].iter().any(|name| std::env::var_os(name).is_some()) {
        assert!(exercised > 0, "a kit variable was set but no saved marker contexts ran");
    }
    eprintln!("{exercised} saved marker field contexts checked in total");
}

/// H4's matching fields have no installed kit yet. Exercise their current
/// layouts with Reach/H2A equivalent paths, creating the necessary block elements.
#[test]
fn halo4_marker_candidates_have_working_picker_annotations() {
    use crate::core::document::apply::apply_one_block_op;
    use crate::core::document::ops::{BlockOp, BlockOpKind};
    let cases: Vec<Case> = serde_json::from_str(include_str!("marker_lookup_cases.json")).unwrap();
    let definitions = locate_definitions_root();
    let root = definitions.join("halo4_mcc");
    let editors = StringEditors::load(&root).unwrap();
    for case in cases.iter().filter(|case| matches!(case.engine.as_str(), "halo2amp_mcc" | "haloreach_mcc") && case.group != "equipment") {
        let mut tag = blam_tags::TagFile::new(root.join(format!("{}.json", case.group))).unwrap();
        let mut segments = Vec::new();
        for segment in case.path.split('/') {
            let clean: String = segment.split('#').next().unwrap().to_owned();
            // Ordinals precede block indices: retain the index while dropping #N.
            let index = segment.find('[').map(|start| &segment[start..]);
            let clean = format!("{}{}", clean.split('[').next().unwrap(), index.unwrap_or(""));
            if let Some((block, rest)) = clean.split_once('[') {
                let index: usize = rest.trim_end_matches(']').parse().unwrap();
                let block_path = segments.iter().cloned().chain([block.to_owned()]).collect::<Vec<_>>().join("/");
                while tag.root().field_path(&block_path).unwrap().as_block().unwrap().len() <= index {
                    apply_one_block_op(&mut tag, &BlockOp { path: block_path.clone(), kind: BlockOpKind::Add }).unwrap();
                }
            }
            segments.push(clean);
        }
        let path = segments.join("/");
        assert_eq!(editors.editor_at(tag.root(), &path), Some("marker_names"), "H4 {path}");
    }
}
