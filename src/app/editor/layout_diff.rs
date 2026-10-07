//! A tag saved with another layout than the definitions give its group: the
//! notice in its header, and the window listing what differs, struct by
//! struct, from blam-tags' [`diff_layouts`].
//!
//! The editor draws a tag's own layout, so a tag saved before fields were
//! added to its group doesn't show them, and saving keeps the layout it has.
//! The notice says so where the tag is named, rather than leaving the user to
//! wonder why a field the definitions have is missing.

use super::*;
use crate::core::game::GameFacts;
use blam_tags::schema_compare::{diff_layouts, FieldChangeKind, LayoutDiff};
use std::sync::{Arc, Mutex, OnceLock};

/// The layout the definitions give `group_name` in `game`, read once.
fn current_layout(definitions_root: &Path, game: GameId, group_name: &str) -> Option<Arc<TagFile>> {
    type Layouts = HashMap<(PathBuf, GameId, String), Option<Arc<TagFile>>>;
    static CACHE: OnceLock<Mutex<Layouts>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry((definitions_root.to_path_buf(), game, group_name.to_owned()))
        .or_insert_with(|| {
            let path = definitions_root.join(game.as_str()).join(format!("{group_name}.json"));
            TagFile::new(path).ok().map(Arc::new)
        })
        .clone()
}

/// How a loaded document's layout differs from the definitions, and a tag of
/// the current layout to draw the fields added since from.
pub(in crate::app) struct LayoutReport {
    pub(in crate::app) diff: LayoutDiff,
    /// A new tag built from the definitions, with an element in every block
    /// on the way to a struct that gained fields, so they can be drawn at
    /// their defaults.
    preview: Option<TagFile>,
}

/// How a loaded document's layout differs from the definitions, worked out
/// once per load (`document`, the document's id, changes when it's reloaded;
/// its layout doesn't change while it's open). `None` when it has the
/// current layout, or there are no definitions to compare it with.
pub(in crate::app) fn document_layout_diff(
    document: u64,
    tag: &TagFile,
    definitions_root: &Path,
    game: GameId,
    group_name: &str,
) -> Option<Arc<LayoutReport>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Option<Arc<LayoutReport>>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Documents closed long ago needn't be remembered; one is cheap to redo.
    if cache.len() > 512 {
        cache.clear();
    }
    cache
        .entry(document)
        .or_insert_with(|| {
            let current = current_layout(definitions_root, game, group_name)?;
            let diff = diff_layouts(tag, &current);
            if diff.is_empty() {
                return None;
            }
            let path = definitions_root.join(game.as_str()).join(format!("{group_name}.json"));
            let preview = TagFile::new(path).ok().map(|mut preview| {
                for struct_diff in &diff.structs {
                    if struct_diff.fields.iter().any(|c| matches!(c.kind, FieldChangeKind::Added { .. })) {
                        add_elements_along(&mut preview, &struct_diff.path);
                    }
                }
                preview
            });
            Some(Arc::new(LayoutReport { diff, preview }))
        })
        .clone()
}

/// The editor path to the struct reached by `names` (a [`StructDiff`] path:
/// field names joined with `/`), through element 0 of each block and array
/// on the way; `None` if a block on the way is empty or a name isn't there.
fn element_path(tag: &TagFile, names: &str) -> Option<String> {
    let mut path = String::new();
    let mut current = tag.root();
    for name in names.split('/').filter(|name| !name.is_empty()) {
        let field = current.fields_all().find(|field| field.name() == name)?;
        let field_path = crate::core::document::value::append_field_path_for(&path, &field);
        if let Some(inner) = field.as_struct() {
            current = inner;
            path = field_path;
        } else if let Some(block) = field.as_block() {
            current = block.element(0)?;
            path = format!("{field_path}[0]");
        } else if let Some(array) = field.as_array() {
            current = array.element(0)?;
            path = format!("{field_path}[0]");
        } else {
            return None;
        }
    }
    Some(path)
}

/// Give every empty block on the way to the struct at `names` an element, so
/// [`element_path`] reaches it.
fn add_elements_along(tag: &mut TagFile, names: &str) {
    let mut reached = String::new();
    for name in names.split('/').filter(|name| !name.is_empty()) {
        let walked = if reached.is_empty() { name.to_owned() } else { format!("{reached}/{name}") };
        if element_path(tag, &walked).is_none() {
            let Some(parent) = element_path(tag, &reached) else { return };
            let root = tag.root();
            let owner = if parent.is_empty() { Some(root) } else { root.descend(&parent) };
            let Some(field) = owner.and_then(|owner| owner.fields_all().find(|field| field.name() == name)) else {
                return;
            };
            let block_path = crate::core::document::value::append_field_path_for(&parent, &field);
            if field.as_block().is_none()
                || crate::core::document::apply::add_block_element(tag, &block_path).is_err()
            {
                return;
            }
        }
        reached = walked;
    }
}

/// What a field drawn in the window needs from the pane: the tag and how
/// the editor reads it.
pub(in crate::app) struct FieldPreview<'a> {
    pub(in crate::app) tag: &'a TagFile,
    pub(in crate::app) names: &'a TagNameIndex,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) definitions_root: &'a Path,
    pub(in crate::app) docs: Option<&'a DefDocs>,
    pub(in crate::app) expert_mode: bool,
    pub(in crate::app) game: GameId,
}

/// Draw the field `name` of the struct at `names` in `tag` through the real
/// field editor, read-only, inside a wash: green for a field added since
/// (from the preview tag, at its default), red for one only this tag has
/// (with its value).
fn draw_field_pane(
    ui: &mut Ui,
    tag: Option<&TagFile>,
    names: &str,
    field_name: &str,
    added: bool,
    preview: &FieldPreview<'_>,
    scope: &str,
) {
    let (wash, accent) = if added {
        (crate::app::browser::added_wash(), crate::app::browser::added_text())
    } else {
        (crate::app::browser::removed_wash(), crate::app::browser::removed_text())
    };
    let target = tag.and_then(|tag| element_path(tag, names).map(|path| (tag, path)));
    Frame::NONE
        .fill(wash)
        .stroke(Stroke::new(1.0_f32, accent.gamma_multiply(0.5)))
        .inner_margin(egui::Margin::symmetric(5, 5))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let Some((tag, path)) = target else {
                ui.label(
                    RichText::new("No element of this tag holds it, so there is no value to show.")
                        .color(subtle_dark())
                        .small(),
                );
                return;
            };
            let root = tag.root();
            let Some(owner) = (if path.is_empty() { Some(root) } else { root.descend(&path) }) else {
                return;
            };
            let Some(field) = owner.fields_all().find(|field| field.name() == field_name) else {
                return;
            };
            // Only this field, and everything inside it.
            let mut visible_paths = std::collections::HashSet::new();
            collect_field_paths(&path, field, &mut visible_paths);
            let filter = FieldFilterAction::Apply(std::sync::Arc::new(FieldFilter { visible_paths }));
            let mut sinks = EditSinks::default();
            let mut edit = FieldEditContext::read_only(&mut sinks, scope, scope);
            edit.expand_all = Some(true);
            edit.nested_default = NestedDefault::Expanded;
            edit.group_tag = preview.group_tag;
            edit.root = Some(root);
            edit.game = Some(preview.game);
            edit.definitions_root = Some(preview.definitions_root);
            edit.names = Some(preview.names);
            edit.docs = preview.docs;
            edit.expert_mode = preview.expert_mode;
            edit.field_filter = Some(&filter);
            egui::ScrollArea::horizontal()
                .id_salt((scope, names, field_name))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    draw_struct_fields_inline(ui, owner, preview.names, 0, preview.expert_mode, &path, &mut edit);
                });
        });
}

/// The canonical (index-free) paths of `field` and everything inside it, as
/// the editor's field filter names them.
fn collect_field_paths(prefix: &str, field: blam_tags::TagField<'_>, out: &mut std::collections::HashSet<String>) {
    let path = crate::core::document::value::append_field_path_for(prefix, &field);
    out.insert(strip_node_indices(&path));
    let mut inside = |inner: TagStruct<'_>, inner_path: &str| {
        for child in inner.fields_all() {
            collect_field_paths(inner_path, child, out);
        }
    };
    if let Some(inner) = field.as_struct() {
        inside(inner, &path);
    } else if let Some(element) = field.as_block().and_then(|block| block.element(0)) {
        inside(element, &format!("{path}[0]"));
    } else if let Some(element) = field.as_array().and_then(|array| array.element(0)) {
        inside(element, &format!("{path}[0]"));
    }
}

/// Which way a tag's layout differs, by which side has fields the other
/// lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutAge {
    /// It lacks fields the definitions have: saved before they were added.
    Older,
    /// It has fields the definitions don't: perhaps from a newer build.
    Newer,
    /// Both, or only sizes and types changed.
    Different,
}

fn layout_age(diff: &LayoutDiff) -> LayoutAge {
    match diff.field_counts() {
        (added, 0) if added > 0 => LayoutAge::Older,
        (0, removed) if removed > 0 => LayoutAge::Newer,
        _ => LayoutAge::Different,
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The header's notice: which way the layout differs and how much.
pub(in crate::app) fn layout_notice_text(diff: &LayoutDiff) -> String {
    let age = match layout_age(diff) {
        LayoutAge::Older => "Older layout",
        LayoutAge::Newer => "Newer layout",
        LayoutAge::Different => "Different layout",
    };
    if diff.structs.is_empty() {
        format!("{age}: group version differs")
    } else {
        format!("{age}: {}", plural(diff.structs.len(), "struct differs", "structs differ"))
    }
}

fn open_id(tag_key: &str) -> egui::Id {
    egui::Id::new(("layout_diff_window", tag_key))
}

/// Draw the notice; a click opens the window for `tag_key`.
pub(in crate::app) fn draw_layout_notice(ui: &mut Ui, tag_key: &str, diff: &LayoutDiff, game: GameId) {
    let response = ui
        .add(
            egui::Label::new(RichText::new(layout_notice_text(diff)).color(READ_ONLY_BADGE_COLOR).strong())
                .sense(Sense::click()),
        )
        .on_hover_text(format!(
            "This tag was saved with a different layout than the {} definitions. Click to see what changed.",
            game.display_name()
        ));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        ui.data_mut(|data| data.insert_temp(open_id(tag_key), true));
    }
}

/// The window, while it's open for `tag_key`.
pub(in crate::app) fn draw_layout_diff_window(
    ctx: &egui::Context,
    tag_key: &str,
    title: &str,
    report: &LayoutReport,
    game: GameId,
    preview: &FieldPreview<'_>,
) {
    let id = open_id(tag_key);
    let mut open = ctx.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    if !open {
        return;
    }
    egui::Window::new(format!("Layout differences: {title}"))
        .id(id.with("window"))
        .open(&mut open)
        .default_size([640.0, 520.0])
        .vscroll(true)
        .show(ctx, |ui| draw_layout_diff(ui, &report.diff, game, Some((report, preview, tag_key))));
    ctx.data_mut(|data| data.insert_temp(id, open));
}

/// What the window shows: what the difference means, then each struct that
/// differs with its fields.
/// With `fields`, the fields added and removed are drawn as the editor draws
/// them beneath their rows.
pub(in crate::app) fn draw_layout_diff(
    ui: &mut Ui,
    diff: &LayoutDiff,
    game_id: GameId,
    fields: Option<(&LayoutReport, &FieldPreview<'_>, &str)>,
) {
    let game = game_id.display_name();
    let explanation = match layout_age(diff) {
        LayoutAge::Older => format!(
            "This tag was saved with an older layout than the {game} definitions. The editor shows the \
             tag's own layout, so fields added since don't appear, and saving keeps the layout it has."
        ),
        LayoutAge::Newer => format!(
            "This tag has fields the {game} definitions don't, and may come from a newer build. The \
             editor shows the tag's own layout, and saving keeps it."
        ),
        LayoutAge::Different => format!(
            "This tag's layout differs from the {game} definitions. The editor shows the tag's own \
             layout, and saving keeps it."
        ),
    };
    ui.label(RichText::new(explanation).color(text_dark()));
    ui.label(
        RichText::new("A renamed field shows as one removed and one added.")
            .color(subtle_dark())
            .small(),
    );
    ui.add_space(6.0);

    let mut counts = [0usize; 4];
    for change in diff.structs.iter().flat_map(|s| &s.fields) {
        match change.kind {
            FieldChangeKind::Added { .. } => counts[0] += 1,
            FieldChangeKind::Removed { .. } => counts[1] += 1,
            FieldChangeKind::Retyped { .. } => counts[2] += 1,
            FieldChangeKind::Moved => counts[3] += 1,
            _ => {}
        }
    }
    let summary = [
        (counts[0], "field added since", "fields added since"),
        (counts[1], "field not in the definitions", "fields not in the definitions"),
        (counts[2], "field retyped", "fields retyped"),
        (counts[3], "field moved", "fields moved"),
    ]
    .into_iter()
    .filter(|(count, _, _)| *count > 0)
    .map(|(count, one, many)| plural(count, one, many))
    .collect::<Vec<_>>();
    let mut heading = plural(diff.structs.len(), "struct differs", "structs differ");
    if !summary.is_empty() {
        heading = format!("{heading}: {}", summary.join(", "));
    }
    ui.label(RichText::new(heading).color(text_dark()).strong());
    if let Some((from, to)) = diff.version {
        ui.label(RichText::new(format!("Group version {from} → {to}")).color(text_dark()));
    }
    ui.add_space(4.0);

    // Open while there are few enough to read at a glance.
    let open = diff.structs.len() <= 8;
    for (index, struct_diff) in diff.structs.iter().enumerate() {
        let place = if struct_diff.path.is_empty() { "root" } else { struct_diff.path.as_str() };
        let size = if struct_diff.size == struct_diff.current_size {
            format!("{} bytes", struct_diff.size)
        } else {
            format!("{} → {} bytes", struct_diff.size, struct_diff.current_size)
        };
        egui::CollapsingHeader::new(RichText::new(format!("{place}  ({}, {size})", struct_diff.current_name)).color(text_dark()))
            .id_salt(("layout_diff_struct", index))
            .default_open(open)
            .show(ui, |ui| {
                if struct_diff.name != struct_diff.current_name {
                    ui.label(
                        RichText::new(format!("Named {} in this tag", struct_diff.name))
                            .color(subtle_dark())
                            .small(),
                    );
                }
                for change in &struct_diff.fields {
                    let (text, color) = match &change.kind {
                        FieldChangeKind::Added { type_name } => {
                            (format!("+ {}  {type_name}, added since", change.name), good_news())
                        }
                        FieldChangeKind::Removed { type_name } => {
                            (format!("− {}  {type_name}, not in the definitions", change.name), REFERENCE_MISSING_COLOR)
                        }
                        FieldChangeKind::Retyped { from, to } => {
                            (format!("~ {}  {from} → {to}", change.name), READ_ONLY_BADGE_COLOR)
                        }
                        FieldChangeKind::Moved => (format!("↕ {}  moved", change.name), subtle_dark()),
                        FieldChangeKind::BlockMaximum { from, to } => {
                            (format!("# {}  block maximum {from} → {to}", change.name), subtle_dark())
                        }
                        FieldChangeKind::ArrayLength { from, to } => {
                            (format!("# {}  array length {from} → {to}", change.name), subtle_dark())
                        }
                    };
                    let offsets = match (change.offset, change.current_offset) {
                        (Some(old), Some(new)) if old != new => format!("Offset {old} → {new}"),
                        (Some(old), _) => format!("Offset {old} in this tag"),
                        (None, Some(new)) => format!("Offset {new} in the definitions"),
                        (None, None) => String::new(),
                    };
                    let row = ui.label(RichText::new(text).color(color).monospace());
                    if !offsets.is_empty() {
                        row.on_hover_text(offsets);
                    }
                    if let Some((report, preview, tag_key)) = fields {
                        let scope = format!("layout_diff|{tag_key}|{index}|{}", change.name);
                        match &change.kind {
                            FieldChangeKind::Added { .. } => draw_field_pane(
                                ui,
                                report.preview.as_ref(),
                                &struct_diff.path,
                                &change.name,
                                true,
                                preview,
                                &scope,
                            ),
                            FieldChangeKind::Removed { .. } => draw_field_pane(
                                ui,
                                Some(preview.tag),
                                &struct_diff.path,
                                &change.name,
                                false,
                                preview,
                                &scope,
                            ),
                            _ => {}
                        }
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::schema_compare::{FieldChange, StructDiff};

    fn painted(ui_fn: impl FnMut(&mut Ui)) -> Vec<String> {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let mut ui_fn = ui_fn;
        let mut texts = Vec::new();
        for _ in 0..2 {
            let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| ui_fn(ui));
            });
            texts = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        texts
    }

    fn sound_mix_like() -> LayoutDiff {
        LayoutDiff {
            version: None,
            structs: vec![StructDiff {
                path: String::new(),
                name: "sound_mix_struct_definition".to_owned(),
                current_name: "sound_mix_block_struct".to_owned(),
                size: 132,
                current_size: 148,
                fields: vec![
                    FieldChange {
                        name: "default transmission settings".to_owned(),
                        kind: FieldChangeKind::Added { type_name: "struct".to_owned() },
                        offset: None,
                        current_offset: Some(0),
                    },
                    FieldChange {
                        name: "first person left side mix".to_owned(),
                        kind: FieldChangeKind::Moved,
                        offset: Some(0),
                        current_offset: Some(16),
                    },
                ],
            }],
        }
    }

    #[test]
    fn the_notice_says_which_way_the_layout_differs() {
        assert_eq!(layout_notice_text(&sound_mix_like()), "Older layout: 1 struct differs");
        let mut newer = sound_mix_like();
        newer.structs[0].fields[0].kind = FieldChangeKind::Removed { type_name: "struct".to_owned() };
        assert!(layout_notice_text(&newer).starts_with("Newer layout"));
        let mut both = sound_mix_like();
        both.structs[0].fields[1].kind = FieldChangeKind::Removed { type_name: "real".to_owned() };
        assert!(layout_notice_text(&both).starts_with("Different layout"));
    }

    /// The window explains the difference and lists each struct's changes.
    #[test]
    fn the_window_lists_what_changed() {
        let texts = painted(|ui| draw_layout_diff(ui, &sound_mix_like(), GameId::HaloReach, None));
        let all = texts.join("\n");
        assert!(all.contains("older layout than the Halo: Reach definitions"), "{all}");
        assert!(all.contains("1 struct differs: 1 field added since, 1 field moved"), "{all}");
        assert!(all.contains("root  (sound_mix_block_struct, 132 → 148 bytes)"), "{all}");
        assert!(all.contains("Named sound_mix_struct_definition in this tag"), "{all}");
        assert!(all.contains("+ default transmission settings  struct, added since"), "{all}");
        assert!(all.contains("↕ first person left side mix  moved"), "{all}");
    }

    /// A shipped Reach `sound_mix` was saved before "default transmission
    /// settings" was added; a tag made from the definitions has the current
    /// layout and gets no notice.
    #[test]
    fn a_shipped_tag_on_an_older_layout_is_found_and_a_new_one_is_not() {
        let definitions = crate::core::bundled::locate_definitions_root();
        let fresh = TagFile::new(definitions.join("haloreach_mcc/sound_mix.json")).unwrap();
        assert!(document_layout_diff(u64::MAX - 1, &fresh, &definitions, GameId::HaloReach, "sound_mix").is_none());

        let tags = crate::core::test_kits::hrek_tags();
        let path = tags.join("sound/sound_mix.sound_mix");
        if !path.is_file() {
            eprintln!("skipping: {} not present", path.display());
            return;
        }
        let shipped = TagFile::read(&path).unwrap();
        let diff = document_layout_diff(u64::MAX - 2, &shipped, &definitions, GameId::HaloReach, "sound_mix")
            .expect("the shipped sound_mix is on an older layout");
        assert_eq!(layout_notice_text(&diff.diff), "Older layout: 1 struct differs");
        assert!(diff.diff.structs[0].fields.iter().any(|c| c.name == "default transmission settings"));
    }
}
