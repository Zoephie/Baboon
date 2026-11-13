//! Preview names first, then apply the reviewed batch through the shared refactor job.
use super::*;
use crate::app::shell::{FolderRefactorUiState, WorkerMessage};
use crate::app::tag_ops::{duplicate::validate_leaf_characters, folder_merge_ops};
use crate::core::script_paths::{ChangeKind, Outcome, ScriptImpact, ScriptReport};

#[derive(Clone)]
pub(super) struct RenameItem {
    pub path: PathBuf,
    pub stem: String,
    pub extension: Option<std::ffi::OsString>,
    /// The tag's group, for its icon. `None` for folders.
    pub group_tag: Option<u32>,
}
#[derive(Clone, PartialEq, Eq)]
/// One Find and Replace covers every rename: `{name}` stands for the current
/// name in both fields, so finding `{name}` replaces whole names (a prefix or
/// suffix is `test_{name}`), and finding literal text replaces just that text.
pub(super) struct RenameOptions {
    pub find: String,
    pub replacement: String,
}
impl Default for RenameOptions {
    fn default() -> Self {
        Self {
            find: "{name}".into(),
            replacement: "{name}".into(),
        }
    }
}
struct PreviewRow {
    target: PathBuf,
    error: Option<String>,
    changed: bool,
}
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// Substitutions never parse the inserted name again: a literal token in an
/// existing filename must remain a literal part of that filename.
pub(in crate::app) fn template(pattern: &str, name: &str) -> Result<String, String> {
    let mut rest = pattern;
    let mut output = String::new();
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("{{") {
            output.push('{');
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("}}") {
            output.push('}');
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("{name}") {
            output.push_str(name);
            rest = tail;
        } else if rest.starts_with('{') {
            return Err("Use {name}; double braces for literal braces".into());
        } else {
            let character = rest.chars().next().unwrap();
            output.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    Ok(output)
}
fn occupied_names(items: &[RenameItem]) -> Result<HashSet<String>, String> {
    let parents: HashSet<_> = items.iter().filter_map(|item| item.path.parent()).collect();
    let mut occupied = HashSet::new();
    for parent in parents {
        for entry in fs::read_dir(parent).map_err(|e| e.to_string())? {
            occupied.insert(path_key(&entry.map_err(|e| e.to_string())?.path()));
        }
    }
    Ok(occupied)
}
/// Tag paths are case-insensitive to Windows and the engines, so Find ignores
/// letter case: `Bugger` finds `bugger`.
fn replace_literal(input: &str, find: &str, replacement: &str) -> String {
    if find.is_empty() {
        return input.to_owned();
    }
    let count = find.chars().count();
    let folded = find.to_lowercase();
    let mut rest = input;
    let mut result = String::new();
    while !rest.is_empty() {
        let end = rest
            .char_indices()
            .nth(count)
            .map(|(index, _)| index)
            .unwrap_or(rest.len());
        if rest[..end].chars().count() == count && rest[..end].to_lowercase() == folded {
            result.push_str(replacement);
            rest = &rest[end..];
        } else {
            let character = rest.chars().next().unwrap();
            result.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    result
}
fn preview(
    items: &[RenameItem],
    options: &RenameOptions,
    occupied: &HashSet<String>,
) -> Vec<PreviewRow> {
    let mut rows = Vec::new();
    for item in items {
        let result = (|| {
            let find = template(&options.find, &item.stem)?;
            let replacement = template(&options.replacement, &item.stem)?;
            // Nothing to find leaves every name as it is.
            let name = replace_literal(&item.stem, &find, &replacement);
            // Unmatched rows stay untouched, including unusual names that the
            // application's naming rules would reject for a newly named tag.
            if name == item.stem {
                return Ok(item.path.clone());
            }
            let name = validate_leaf_characters(&name, "Names", "Enter a name")?;
            let mut leaf = std::ffi::OsString::from(name);
            if let Some(extension) = &item.extension {
                leaf.push(".");
                leaf.push(extension);
            }
            let target = item.path.with_file_name(leaf);
            if path_key(&target) == path_key(&item.path) {
                return Err("Only the letter case changed; choose a different name".into());
            }
            if occupied.contains(&path_key(&target)) {
                return Err("That name already exists in this folder".into());
            }
            Ok(target)
        })();
        rows.push(match result {
            Ok(target) => PreviewRow {
                changed: target != item.path,
                target,
                error: None,
            },
            Err(error) => PreviewRow {
                target: item.path.clone(),
                changed: false,
                error: Some(error),
            },
        });
    }
    let mut targets = HashMap::<String, Vec<usize>>::new();
    for (index, row) in rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.error.is_none())
    {
        targets
            .entry(path_key(&row.target))
            .or_default()
            .push(index);
    }
    for group in targets.values().filter(|group| group.len() > 1) {
        for &index in group {
            rows[index].error = Some("Several selected items would have this name".into());
        }
    }
    rows
}

/// Known referrers for the entire selection, available before editing names.
struct RenameReferences {
    signature: (usize, usize, bool),
    complete: bool,
    /// Display path and tag group of each known referring tag.
    paths: Vec<(String, u32)>,
    /// For each selected folder, the tags beneath it, which get new paths
    /// when it is renamed. Zero for selected tags.
    inside: Vec<usize>,
}
impl RenameReferences {
    fn signature(source: &LoadedSourceData) -> Option<(usize, usize, bool)> {
        Some((
            source.reverse_dependencies.as_ref()?.len(),
            source.full_entry_set().len(),
            source.complete_scan,
        ))
    }
    fn new(items: &[RenameItem], source: &LoadedSourceData) -> Option<Self> {
        let index = source.reverse_dependencies.as_ref()?;
        let entries = source.full_entry_set();
        let owners: HashMap<_, _> = items
            .iter()
            .enumerate()
            .map(|(i, item)| (path_key(&item.path), i))
            .collect();
        let mut keys = vec![HashSet::new(); items.len()];
        let mut inside = vec![0; items.len()];
        for entry in entries {
            let TagEntryLocation::LooseFile(path) = &entry.location else {
                continue;
            };
            let selected: Vec<_> = path
                .ancestors()
                .filter_map(|parent| owners.get(&path_key(parent)).copied())
                .collect();
            if selected.is_empty() {
                continue;
            }
            for &item in &selected {
                if items[item].extension.is_none() {
                    inside[item] += 1;
                }
            }
            let Some(reference) =
                crate::app::references::dependency_entry_reference_path(entry, &source.names)
            else {
                continue;
            };
            for item in selected {
                keys[item].extend(
                    index
                        .dependents_for(entry.group_tag, &reference)
                        .iter()
                        .map(String::as_str),
                );
            }
        }
        let display: HashMap<_, _> = entries
            .iter()
            .map(|entry| (entry.key.as_str(), (entry.display_path.as_str(), entry.group_tag)))
            .collect();
        let mut complete =
            source.complete_scan && entries.iter().all(|entry| index.contains_tag(&entry.key));
        let by_item: Vec<Vec<(String, u32)>> = keys
            .into_iter()
            .map(|keys| {
                keys.into_iter()
                    .filter_map(|key| {
                        if let Some((path, group)) = display.get(key) {
                            Some(((*path).to_owned(), *group))
                        } else {
                            complete = false;
                            None
                        }
                    })
                    .collect()
            })
            .collect();
        let mut paths: Vec<_> = by_item
            .into_iter()
            .flatten()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        paths.sort_by_cached_key(|(path, _)| crate::core::source::natural_key(path));
        Some(Self {
            signature: Self::signature(source)?,
            complete,
            paths,
            inside,
        })
    }
    /// The boxed "N References to Fix" list: each referring tag with its
    /// icon and a muted folder path, scrolling within `height`.
    /// Like the list, the note covers every selected folder: the list shows
    /// what the rename could reach, and applying renames only the items whose
    /// names change, checking their references again first.
    fn draw(ui: &mut Ui, references: Option<&Self>, scripts: &ScriptRows, refreshing: bool, game: Option<GameId>, height: f32) {
        let mut notes = Vec::new();
        let moved: usize = references.map_or(0, |references| references.inside.iter().sum());
        if moved > 0 {
            notes.push(folder_tags_note(moved, "renamed folders"));
        }
        match references {
            None => notes.push("Reference list is not ready yet. References are still checked and fixed when renaming.".to_owned()),
            Some(_) if refreshing => notes.push("Refreshing references; showing the last known list.".to_owned()),
            Some(references) if !references.complete => notes.push("This list is incomplete; it shows the references currently known.".to_owned()),
            _ => {}
        }
        let paths = references.map_or(&[][..], |references| &references.paths[..]);
        draw_references_box(ui, &references_to_fix_title(scripts.total(paths.len())), paths, scripts, &notes, game, height);
    }
}

const ROW_ICON_SIZE: f32 = 18.0;
const REFERENCE_ROW_HEIGHT: f32 = 28.0;
const PREVIEW_ROW_HEIGHT: f32 = 32.0;
pub(in crate::app) const ITEM_ROW_HEIGHT: f32 = 32.0;

/// The selection footer: a button-height row with 6px above and below, like
/// the Folder and String tables' footers.
const SELECTION_FOOTER_HEIGHT: f32 = BUTTON_HEIGHT + 12.0;
/// The corners of every Rename and Delete box, matching the tables'.
const BOX_RADIUS: u8 = crate::app::editor::fields::FOUNDATION_CONTAINER_RADIUS as u8;

/// The note under References to Fix when renamed folders hold tags: their
/// paths change too. `within` names the folders, e.g. "renamed folders".
pub(in crate::app) fn folder_tags_note(tags: usize, within: &str) -> String {
    if tags == 1 {
        format!("Includes references to 1 tag within {within} that will now have a new path.")
    } else {
        format!("Includes references to {tags} tags within {within} that will now have new paths.")
    }
}

/// The height a references box needs to show `rows` rows without scrolling,
/// capped at `max`: its title, its notes, and the rows.
pub(in crate::app) fn references_box_height(rows: usize, notes: usize, max: f32) -> f32 {
    let title = 52.0 + notes as f32 * 40.0;
    (title + rows as f32 * REFERENCE_ROW_HEIGHT).min(max)
}

/// The Rename windows' references title: "N References to Fix".
pub(in crate::app) fn references_to_fix_title(count: usize) -> String {
    match count {
        0 => "No References to Fix".to_owned(),
        1 => "1 Reference to Fix".to_owned(),
        count => format!("{count} References to Fix"),
    }
}

/// The Delete windows' references title: the tags a delete leaves pointing
/// at something missing.
pub(in crate::app) fn missing_references_title(count: usize) -> String {
    match count {
        0 => "No Tags Will Have Missing References".to_owned(),
        1 => "1 Tag Will Have Missing References".to_owned(),
        count => format!("{count} Tags Will Have Missing References"),
    }
}

/// One item as a row: its icon, then its path with the folder muted.
pub(in crate::app) fn draw_item_row(ui: &mut Ui, icon: RenameIcon, game: Option<GameId>, path: &str, height: f32) {
    let (row, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let icon_rect = egui::Rect::from_center_size(egui::pos2(row.left() + 18.0, row.center().y), Vec2::splat(ROW_ICON_SIZE));
    match icon {
        RenameIcon::Folder => paint_button_icon_at(ui, ButtonIcon::FolderClosed, icon_rect, text_dark()),
        RenameIcon::Tag(group) => paint_tag_icon_at(ui, group, game, icon_rect),
    }
    let text = egui::Rect::from_min_max(egui::pos2(icon_rect.right() + 10.0, row.top()), row.max);
    if crate::app::compare::tag_compare::paint_path_label(ui, path, text) {
        response.on_hover_text(path);
    }
    ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, foundation_block_edge()));
}

/// The boxed list of referring tags shared by the Rename and Delete windows:
/// `title`, any notes, then each referring tag with its icon and a muted
/// folder path, then each affected script, scrolling within `height`. `rows`
/// are display paths and tag groups. The title counts both; build it with
/// [`ScriptRows::total`].
pub(in crate::app) fn draw_references_box(
    ui: &mut Ui,
    title: &str,
    rows: &[(String, u32)],
    scripts: &ScriptRows,
    notes: &[String],
    game: Option<GameId>,
    height: f32,
) {
    let notes: Vec<String> = notes.iter().chain(&scripts.notes).cloned().collect();
    boxed_rows(ui, "rename_references", title, &notes, scripts.checking, rows.len() + scripts.rows.len(), height, |ui, row| {
        match rows.get(row) {
            Some((path, group)) => draw_item_row(ui, RenameIcon::Tag(Some(*group)), game, path, REFERENCE_ROW_HEIGHT),
            None => draw_script_row(ui, &scripts.rows[row - rows.len()], game),
        }
    });
}

/// The scripts a references box lists after its tags, and the notes about
/// them. Empty when no script is affected.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::app) struct ScriptRows {
    pub rows: Vec<ScriptRow>,
    pub notes: Vec<String>,
    /// The check is still running: the box shows a spinner, and the window
    /// holds its Rename or Delete button until it finishes.
    pub checking: bool,
}

/// One affected script.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct ScriptRow {
    /// `scenarios\solo\03a\scripts\03a_mission.hsc`, folder muted when drawn.
    pub path: String,
    /// "12 paths", at the right.
    pub count: String,
    /// "· 3 can't fix" after the count, in the error colour.
    pub problem: Option<String>,
    /// The affected lines, shown on hover.
    pub tooltip: String,
}

/// How many lines a script row's tooltip lists before "+N more".
const SCRIPT_TOOLTIP_LINES: usize = 20;

impl ScriptRows {
    /// The line beside the spinner while the script check runs.
    pub const CHECKING: &'static str = "Checking scripts…";
    /// The hint on a Rename or Delete button held for the check.
    pub const WAITING: &'static str = "Available once the script check finishes.";

    /// A box's row count: its tags plus these scripts.
    pub fn total(&self, tags: usize) -> usize {
        tags + self.rows.len()
    }

    /// How many lines these add under the box's title: the notes, and the
    /// spinner while checking.
    pub fn note_lines(&self) -> usize {
        self.notes.len() + usize::from(self.checking)
    }

    /// The rows for `report`, or (`None`) a spinner while scripts are still
    /// being checked. Games whose scripts Baboon does not check get the note
    /// saying so instead.
    pub fn new(game: Option<GameId>, kind: ChangeKind, report: Option<&ScriptReport>) -> Self {
        use crate::core::script_paths::{NOT_CHECKED_NOTE, ScriptSupport, script_support};
        match script_support(game) {
            None => return Self::default(),
            Some(ScriptSupport::NotChecked) => {
                return Self { rows: Vec::new(), notes: vec![NOT_CHECKED_NOTE.to_owned()], checking: false };
            }
            Some(ScriptSupport::Checked) => {}
        }
        let Some(report) = report else {
            return Self { rows: Vec::new(), notes: Vec::new(), checking: true };
        };
        let rows: Vec<_> = report.scripts.iter().map(|script| script_row(script, kind)).collect();
        let notes = if rows.is_empty() {
            Vec::new()
        } else if kind == ChangeKind::Delete {
            vec!["These scripts won't compile until the paths are fixed, so their missions load without scripts.".to_owned()]
        } else {
            vec!["Scripts are updated in their .hsc file in data\\ and in the copy saved in their scenario.".to_owned()]
        };
        Self { rows, notes, checking: false }
    }
}

fn plural_paths(count: usize) -> String {
    if count == 1 { "1 path".to_owned() } else { format!("{count} paths") }
}

fn script_row(script: &ScriptImpact, kind: ChangeKind) -> ScriptRow {
    let (main, problem) = match kind {
        ChangeKind::Rename => (Outcome::Fixed, Outcome::CantFix),
        ChangeKind::Delete => (Outcome::Missing, Outcome::MayBreak),
    };
    let problems = script.count(problem);
    let problem = (problems > 0).then(|| match kind {
        ChangeKind::Rename => format!("· {problems} can't fix"),
        ChangeKind::Delete => format!("· {problems} may break"),
    });
    let mut tooltip = vec![script.path()];
    for hit in script.hits.iter().take(SCRIPT_TOOLTIP_LINES) {
        let what = match (hit.outcome, &hit.new) {
            (Outcome::Fixed, Some(new)) => format!("{} → {new}", hit.old),
            (Outcome::Fixed, None) => format!("{} (updated to the new path)", hit.old),
            (Outcome::CantFix, _) => format!("{} — can't fix: it could mean more than one tag", hit.old),
            (Outcome::Missing, _) => format!("{} — missing", hit.old),
            (Outcome::MayBreak, _) => format!("{} — may be missing: it could mean more than one tag", hit.old),
        };
        let expected = hit.expected.as_deref().map(|expected| format!(" ({expected})")).unwrap_or_default();
        tooltip.push(format!("Line {}: {what}{expected}", hit.line));
    }
    if script.hits.len() > SCRIPT_TOOLTIP_LINES {
        tooltip.push(format!("+{} more", script.hits.len() - SCRIPT_TOOLTIP_LINES));
    }
    ScriptRow { path: script.path(), count: plural_paths(script.count(main)), problem, tooltip: tooltip.join("\n") }
}

/// The script source tag group, for the script rows' icon.
const SCRIPT_GROUP: u32 = u32::from_be_bytes(*b"hsc*");

/// A script as a row: the script icon, its path with the folder muted, and
/// how many of its paths change at the right. Hover lists the lines.
fn draw_script_row(ui: &mut Ui, script: &ScriptRow, game: Option<GameId>) {
    let (row, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), REFERENCE_ROW_HEIGHT), Sense::hover());
    let icon_rect = egui::Rect::from_center_size(egui::pos2(row.left() + 18.0, row.center().y), Vec2::splat(ROW_ICON_SIZE));
    paint_tag_icon_at(ui, Some(SCRIPT_GROUP), game, icon_rect);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut right = row.right() - 8.0;
    if let Some(problem) = &script.problem {
        let galley = ui.painter().layout_no_wrap(problem.clone(), font.clone(), ui.visuals().error_fg_color);
        right -= galley.size().x;
        ui.painter().galley(egui::pos2(right, row.center().y - galley.size().y * 0.5), galley, ui.visuals().error_fg_color);
        right -= 6.0;
    }
    let galley = ui.painter().layout_no_wrap(script.count.clone(), font, subtle_dark());
    right -= galley.size().x;
    ui.painter().galley(egui::pos2(right, row.center().y - galley.size().y * 0.5), galley, subtle_dark());
    let text = egui::Rect::from_min_max(egui::pos2(icon_rect.right() + 10.0, row.top()), egui::pos2(right - 12.0, row.bottom()));
    crate::app::compare::tag_compare::paint_path_label(ui, &script.path, text);
    response.on_hover_text(&script.tooltip);
    ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, foundation_block_edge()));
}

/// The same box for a mix of folders and files, such as the items a bulk
/// delete removes. `id` keeps its scroll position apart from other boxes.
pub(in crate::app) fn draw_items_box(ui: &mut Ui, id: &str, title: &str, rows: &[(String, RenameIcon)], notes: &[String], game: Option<GameId>, height: f32) {
    boxed_rows(ui, id, title, notes, false, rows.len(), height, |ui, row| {
        let (path, icon) = &rows[row];
        draw_item_row(ui, *icon, game, path, REFERENCE_ROW_HEIGHT);
    });
}

#[allow(clippy::too_many_arguments)]
fn boxed_rows(ui: &mut Ui, id: &str, title: &str, notes: &[String], checking: bool, rows: usize, height: f32, mut draw: impl FnMut(&mut Ui, usize)) {
    rename_box(ui, height, |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(title).strong());
        for note in notes {
            ui.label(RichText::new(note).italics().color(subtle_dark()));
        }
        if checking {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(ScriptRows::CHECKING).italics().color(subtle_dark()));
            });
        }
        ui.add_space(4.0);
        if rows == 0 { return }
        // `show_rows` sizes its content from the spacing in force here, so
        // zero it first or a list that fits still scrolls.
        ui.spacing_mut().item_spacing.y = 0.0;
        ScrollArea::vertical()
            .id_salt(id)
            .auto_shrink([false, false])
            .show_rows(ui, REFERENCE_ROW_HEIGHT, rows, |ui, visible| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in visible {
                    draw(ui, row);
                }
            });
    });
}

/// The band along the bottom of the preview box: how many items are selected,
/// and which of them the rename changes.
/// Styled like the Folder and String tables' footers, and joined to the box
/// above it.
fn draw_selection_footer(ui: &mut Ui, selected: usize, folders: usize, tags: usize, invalid: usize) {
    let plural = |count: usize, one: &str, many: &str| format!("{count} {}", if count == 1 { one } else { many });
    let mut job = egui::text::LayoutJob::default();
    let style = ui.style().clone();
    let mut add = |text: &str, color: Color32| RichText::new(text).color(color)
        .append_to(&mut job, &style, egui::FontSelection::Default, egui::Align::Center);
    add(&plural(selected, "item selected", "items selected"), subtle_dark());
    add(" · ", subtle_dark());
    let parts: Vec<String> = [(folders, "folder", "folders"), (tags, "tag", "tags")].into_iter()
        .filter(|(count, ..)| *count > 0)
        .map(|(count, one, many)| plural(count, one, many))
        .collect();
    if parts.is_empty() {
        add("nothing to rename yet", subtle_dark());
    } else {
        add(&format!("renaming {}", parts.join(" & ")), foundation_blue());
    }
    if invalid > 0 {
        add(" · ", subtle_dark());
        add(&plural(invalid, "invalid name", "invalid names"), ui.visuals().error_fg_color);
    }
    let background = ui.painter().add(egui::Shape::Noop);
    let footer = egui::Frame::NONE
        .inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_height(BUTTON_HEIGHT);
                ui.add(egui::Label::new(job).truncate());
            });
        });
    // Its top edge lies on the box's bottom border, so the two share one line.
    let rect = footer.response.rect;
    ui.painter().set(background, egui::epaint::RectShape::new(
        rect.with_min_y(rect.min.y - 1.0),
        egui::CornerRadius { nw: 0, ne: 0, sw: BOX_RADIUS, se: BOX_RADIUS },
        foundation_block_bar(),
        Stroke::new(1.0, foundation_block_edge()),
        egui::StrokeKind::Inside,
    ));
}

/// Rename and Delete windows inset their content this far from the window's
/// sides.
pub(in crate::app) const CONTENT_SIDE_PADDING: i8 = 20;

/// The body of a Rename or Delete window, between its header and its footer
/// buttons, inset [`CONTENT_SIDE_PADDING`] from the window's sides.
pub(in crate::app) fn padded_body<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let margin = ui.spacing().window_margin;
    let side = |window: i8| (CONTENT_SIDE_PADDING - window).max(0);
    egui::Frame::NONE
        .inner_margin(egui::Margin { left: side(margin.left), right: side(margin.right), top: 0, bottom: 0 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            contents(ui)
        })
        .inner
}

/// What a rename row's icon shows: the browser's closed folder, or a tag
/// group's own icon.
#[derive(Clone, Copy)]
pub(in crate::app) enum RenameIcon {
    Folder,
    Tag(Option<u32>),
}

/// The Original | Rename heading of a rename table `width` wide. Where the
/// Rename column starts, from the table's left edge.
pub(in crate::app) fn rename_table_header(ui: &mut Ui, width: f32) -> f32 {
    let column = (width / 2.0).floor();
    let (header, _) = ui.allocate_exact_size(Vec2::new(width, TABLE_HEADER_HEIGHT + 6.0), Sense::hover());
    for (offset, title) in [(0.0, "Original"), (column, "Rename")] {
        ui.painter().text(header.left_center() + Vec2::new(offset + 8.0, 0.0), egui::Align2::LEFT_CENTER,
            title, table_header_font(ui), subtle_dark());
    }
    ui.painter().vline(header.left() + column, header.y_range(), Stroke::new(1.0, foundation_block_edge()));
    paint_table_header_rule(ui, header);
    column
}

/// One row of a rename table: the icon and original name, then the result
/// in `color` (blue when it changes, red for a problem).
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn rename_table_row(
    ui: &mut Ui, width: f32, column: f32, icon: RenameIcon, game: Option<GameId>,
    original: &str, original_hover: &str, renamed: &str, color: Color32,
) {
    let (line, _) = ui.allocate_exact_size(Vec2::new(width, PREVIEW_ROW_HEIGHT), Sense::hover());
    let icon_rect = egui::Rect::from_center_size(egui::pos2(line.left() + 8.0 + ROW_ICON_SIZE * 0.5, line.center().y), Vec2::splat(ROW_ICON_SIZE));
    match icon {
        RenameIcon::Folder => paint_button_icon_at(ui, ButtonIcon::FolderClosed, icon_rect, text_dark()),
        RenameIcon::Tag(group) => paint_tag_icon_at(ui, group, game, icon_rect),
    }
    let cells = [
        (egui::Rect::from_min_max(egui::pos2(icon_rect.right() + 10.0, line.top()), egui::pos2(line.left() + column - 4.0, line.bottom())), original, text_dark(), original_hover),
        (egui::Rect::from_min_max(egui::pos2(line.left() + column + 8.0, line.top()), egui::pos2(line.right() - 4.0, line.bottom())), renamed, color, renamed),
    ];
    for (rect, text, color, hover) in cells {
        let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        cell.add(egui::Label::new(RichText::new(text).color(color)).truncate()).on_hover_text(hover);
    }
    ui.painter().hline(line.x_range(), line.bottom() - 0.5, Stroke::new(1.0, foundation_block_edge()));
}

/// Display paths with their tag groups, read from each path's extension.
pub(in crate::app) fn referrer_rows(paths: &[String], names: &crate::core::format::TagNameIndex) -> Vec<(String, u32)> {
    paths.iter().map(|path| {
        let group = path.rsplit_once('.').and_then(|(_, extension)| names.group_tag_for(extension)).unwrap_or(0);
        (path.clone(), group)
    }).collect()
}

/// The outlined, rounded containers that hold the references and the preview.
pub(in crate::app) fn rename_box(ui: &mut Ui, height: f32, contents: impl FnOnce(&mut Ui)) {
    framed_box(ui, height, egui::CornerRadius::same(BOX_RADIUS), contents);
}

fn framed_box(ui: &mut Ui, height: f32, radius: egui::CornerRadius, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, foundation_block_edge()))
        .corner_radius(radius)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height((height - 16.0).max(0.0));
            contents(ui);
        });
}

/// `folder` on a band running edge to edge under a window's header, as Bulk
/// Rename shows the folder it renames in and Move the folder it moves to.
pub(in crate::app) fn draw_folder_band(ui: &mut Ui, folder: &str) {
    let (band, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::hover());
    let margin = ui.spacing().window_margin;
    let fill = egui::Rect::from_min_max(egui::pos2(band.left() - f32::from(margin.left), band.top() - ui.spacing().item_spacing.y * 0.5),
        egui::pos2(band.right() + f32::from(margin.right), band.bottom()));
    ui.painter().rect_filled(fill, 0.0, foundation_group_bg());
    let icon = egui::Rect::from_center_size(egui::pos2(band.left() + 2.0 + ROW_ICON_SIZE * 0.5, band.center().y), Vec2::splat(ROW_ICON_SIZE));
    paint_button_icon_at(ui, ButtonIcon::FolderOpen, icon, text_dark());
    let text = egui::Rect::from_min_max(egui::pos2(icon.right() + 10.0, band.top()), band.max);
    crate::app::compare::tag_compare::paint_path_label(ui, folder, text);
}

/// `folder` as a display path under the tags root, or the root's own name.
pub(in crate::app) fn folder_display(folder: &Path, root: &Path) -> String {
    match folder.strip_prefix(root) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative.to_string_lossy().replace('/', "\\"),
        _ => root.file_name().unwrap_or(root.as_os_str()).to_string_lossy().into_owned(),
    }
}

/// The folder the selection lives in, as a display path under the kit's root:
/// the deepest folder holding every selected item, or the root's own name.
fn selection_folder(items: &[RenameItem], root: &Path) -> String {
    let mut common: Option<PathBuf> = None;
    for parent in items.iter().filter_map(|item| item.path.parent()) {
        common = Some(match common {
            None => parent.to_path_buf(),
            Some(common) => common.ancestors().find(|ancestor| parent.starts_with(ancestor)).unwrap_or(Path::new("")).to_path_buf(),
        });
    }
    folder_display(&common.unwrap_or_default(), root)
}

pub(in crate::app) struct BulkRenameDialog {
    request: bulk::BulkRequest,
    source_identity: (PathBuf, Option<GameId>),
    items: Vec<RenameItem>,
    occupied: HashSet<String>,
    options: RenameOptions,
    references: Option<RenameReferences>,
    references_revision: Option<(u64, bool)>,
    /// Scripts that name the selected tags, checked once in the background.
    scripts: Option<crate::app::tag_ops::script_fixes::ScriptCheck>,
    error: Option<String>,
    focus: bool,
}
impl BulkRenameDialog {
    /// Index refreshes advance generation without replacing the editing kit.
    /// Stay attached to its stable identity, root and engine; apply revalidates
    /// the selected paths and reference index before changing any files.
    fn kit_index(&self, model: &Model) -> Option<usize> {
        let index = model.kit_index(self.request.stamp.kit)?;
        let source = model.kits[index].source.as_ref()?;
        match &source.source {
            TagSource::LooseFolder { root, game, .. }
                if root == &self.source_identity.0 && game == &self.source_identity.1 =>
            {
                Some(index)
            }
            _ => None,
        }
    }
}

fn insert_token(ui: &Ui, id: egui::Id, text: &mut String, token: &str) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let range = state.cursor.char_range();
    let (start, end) = range
        .map(|range| {
            (
                usize::from(range.primary.index.min(range.secondary.index)),
                usize::from(range.primary.index.max(range.secondary.index)),
            )
        })
        .unwrap_or((text.chars().count(), text.chars().count()));
    let byte = |index| {
        text.char_indices()
            .nth(index)
            .map(|(index, _)| index)
            .unwrap_or(text.len())
    };
    text.replace_range(byte(start)..byte(end), token);
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(start + token.chars().count()),
        )));
    state.store(ui.ctx(), id);
    ui.memory_mut(|memory| memory.request_focus(id));
}
impl Dialog for BulkRenameDialog {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut open = true;
        let previous_options = self.options.clone();
        let rows = preview(&self.items, &self.options, &self.occupied);
        let changes = rows
            .iter()
            .filter(|row| row.changed && row.error.is_none())
            .count();
        let invalid = rows.iter().filter(|row| row.error.is_some()).count();
        let kit_index = self.kit_index(cx.model);
        let mut refreshing_references = false;
        if let Some(index) = kit_index {
            let kit = &cx.model.kits[index];
            let source = kit.source.as_ref().unwrap();
            let revision = (kit.generation, kit.index_jobs.building_references);
            if let Some(signature) = RenameReferences::signature(source) {
                if self.references_revision != Some(revision)
                    || self.references.as_ref().map(|references| references.signature) != Some(signature)
                {
                    self.references = RenameReferences::new(&self.items, source);
                    self.references_revision = Some(revision);
                }
            } else {
                // Keep the last known list while a new index is being built.
                self.references_revision = None;
            }
            refreshing_references = source.reverse_dependencies.is_none()
                || kit.index_jobs.building_references;
        } else {
            self.references = None;
            self.references_revision = None;
        }
        // Every tag in the selection could get a new path. The new names are
        // still being typed, so the rows say how many paths each script has.
        let selected: Vec<PathBuf> = self.items.iter().map(|item| item.path.clone()).collect();
        let scripts = if kit_index.is_some() {
            crate::app::tag_ops::script_fixes::ScriptCheck::rows_for(&mut self.scripts, cx, self.request.stamp.kit, |entries| {
                crate::core::script_paths::ScriptChanges::renaming(entries.iter().filter_map(|entry| match &entry.location {
                    TagEntryLocation::LooseFile(path) if selected.iter().any(|item| path.starts_with(item)) => {
                        Some((entry.display_path.as_str(), None))
                    }
                    _ => None,
                }))
            })
        } else {
            ScriptRows::default()
        };
        let dirty = kit_index.is_some_and(|index| cx.model.kits[index]
            .parsed_tags.values().any(|doc| doc.dirty.is_set()));
        let game = self.source_identity.1;
        let folder = selection_folder(&self.items, &self.source_identity.0);
        let modal=egui::Modal::new(egui::Id::unique("bulk_rename_items"))
            .frame(egui::Frame::window(&cx.egui.global_style())).show(cx.egui,|ui| {
                ui.set_width(window_width(cx.egui,880.0));
                if crate::app::search::draw_icon_window_header_with_help(ui,"Bulk Rename",ButtonIcon::Rename,&mut open,
                    ("About Renaming",egui::Id::unique("bulk_rename_about"))) {
                    cx.send(crate::app::help::HelpCommand::ShowDocSection(crate::app::help::RENAMING_HELP_SECTION));
                }
                // The folder being renamed in.
                draw_folder_band(ui, &folder);
                ui.add_space(14.0);
                padded_body(ui, |ui| {
                let body_height = (window_work_area(cx.egui).height() - 260.0).clamp(300.0, 440.0);
                let gap = 20.0;
                let column_width = ((ui.available_width() - gap) / 2.0).floor();
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    // Left: Find and Replace With, then the references to fix.
                    ui.allocate_ui_with_layout(Vec2::new(column_width, body_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(column_width);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let find_id=egui::Id::unique(("bulk_rename_find",self.request.stamp.kit));
                        let replace_id=egui::Id::unique(("bulk_rename_replace",self.request.stamp.kit));
                        ui.label("Find");
                        ui.add(crate::app::ui_kit::singleline_text_edit(&mut self.options.find).id(find_id).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                        if ui.button("Current Name").on_hover_text("Insert {name}, the current name, into Find").clicked() {
                            insert_token(ui,find_id,&mut self.options.find,"{name}");
                        }
                        ui.add_space(8.0);
                        ui.label("Replace With");
                        let replace=ui.add(crate::app::ui_kit::singleline_text_edit(&mut self.options.replacement).id(replace_id).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                        if self.focus {replace.request_focus();self.focus=false;}
                        if ui.button("Current Name").on_hover_text("Insert {name}, the current name, into Replace With").clicked() {
                            insert_token(ui,replace_id,&mut self.options.replacement,"{name}");
                        }
                        if let Some(error)=&self.error {ui.colored_label(ui.visuals().error_fg_color,error);}
                        if kit_index.is_none() {
                            ui.colored_label(ui.visuals().error_fg_color, "The workspace changed. Reopen Rename for this selection.");
                        }
                        ui.add_space(12.0);
                        let remaining = ui.available_rect_before_wrap().height().max(120.0);
                        RenameReferences::draw(ui,self.references.as_ref(),&scripts,refreshing_references,game,remaining);
                    });
                    // Right: each item's Original name beside its Rename result.
                    ui.allocate_ui_with_layout(Vec2::new(column_width, body_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(column_width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let square_bottom = egui::CornerRadius { nw: BOX_RADIUS, ne: BOX_RADIUS, sw: 0, se: 0 };
                        framed_box(ui, body_height - SELECTION_FOOTER_HEIGHT, square_bottom, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let width = ui.available_width();
                            let column = rename_table_header(ui, width);
                            let list_height = ui.available_height().max(PREVIEW_ROW_HEIGHT);
                            ScrollArea::vertical().id_salt("rename_preview").max_height(list_height).auto_shrink([false,false]).show_rows(ui,PREVIEW_ROW_HEIGHT,rows.len(),|ui,visible| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for index in visible {
                                    let item=&self.items[index];let row=&rows[index];
                                    let label=row.error.as_deref().map(str::to_owned).unwrap_or_else(||row.target.file_name().unwrap_or_default().to_string_lossy().into_owned());
                                    let color=if row.error.is_some(){ui.visuals().error_fg_color}else if row.changed{foundation_blue()}else{subtle_dark()};
                                    let icon = if item.extension.is_none() { RenameIcon::Folder } else { RenameIcon::Tag(item.group_tag) };
                                    let original = item.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                                    rename_table_row(ui, width, column, icon, game, &original, &item.path.display().to_string(), &label, color);
                                }
                            });
                        });
                        let renaming = |folders: bool| rows.iter().zip(&self.items)
                            .filter(|(row, item)| row.changed && row.error.is_none() && item.extension.is_none() == folders)
                            .count();
                        draw_selection_footer(ui, rows.len(), renaming(true), renaming(false), invalid);
                    });
                });
                });
                ui.add_space(12.0);
                popup_footer(ui,|ui| {ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        if icon_text_button(ui,ButtonIcon::Clear,"Cancel",true).clicked(){open=false;}
                        let rename = icon_text_button(ui,ButtonIcon::Rename,if dirty{"Save All and Rename"}else{"Rename"},invalid==0&&changes>0&&kit_index.is_some()&&!scripts.checking);
                        let rename = if invalid > 0 {
                            rename.on_disabled_hover_text(format!("{invalid} invalid {}; see the Rename column", if invalid == 1 { "name" } else { "names" }))
                        } else if scripts.checking {
                            rename.on_disabled_hover_text(ScriptRows::WAITING)
                        } else { rename };
                        if rename.clicked() {
                            cx.send(BrowserCommand::ApplyBulkRename {save:dirty});
                        }
                    });
                });});
            });
        if previous_options != self.options {
            self.error = None;
        }
        open && !modal.should_close()
    }
}

impl Baboon {
    pub(in crate::app) fn open_bulk_rename(&mut self, request: bulk::BulkRequest) {
        let result = (|| {
            let index = self
                .model
                .resolve_stamp(request.stamp)
                .ok_or("The workspace changed; select the items again")?;
            if self.model.editing_kit_is_read_only(index) {
                return Err("This editing kit is read-only".into());
            }
            let source = self.model.kits[index]
                .source
                .as_ref()
                .ok_or("No workspace is loaded")?;
            let TagSource::LooseFolder { root, game, .. } = &source.source else {
                return Err("Bulk rename requires loose tags and folders".into());
            };
            let mut items = Vec::new();
            for key in &request.keys {
                let folder = key.strip_prefix(FOLDER_PANE_PREFIX);
                let (path, group_tag) = if let Some(path) = folder {
                    (root.join(tag_path::to_relative(path)), None)
                } else {
                    match source.entry_for_key(key) {
                        Some(entry) => match &entry.location {
                            TagEntryLocation::LooseFile(path) => (path.clone(), Some(entry.group_tag)),
                            _ => return Err("Every selected item must support Rename".into()),
                        },
                        None => return Err("Every selected item must support Rename".into()),
                    }
                };
                if path == *root || !path.starts_with(root) || !path.exists() {
                    return Err("A selected path is invalid or missing".into());
                }
                let stem = if folder.is_some() {
                    path.file_name()
                } else {
                    path.file_stem()
                }
                .ok_or("A selected item has no name")?
                .to_string_lossy()
                .into_owned();
                let extension = if folder.is_some() {
                    None
                } else {
                    path.extension().map(std::ffi::OsStr::to_os_string)
                };
                items.push(RenameItem {
                    path,
                    stem,
                    extension,
                    group_tag,
                });
            }
            if items.is_empty() {
                return Err("Select items to rename".into());
            }
            let occupied = occupied_names(&items)?;
            Ok(BulkRenameDialog {
                request,
                source_identity: (root.clone(), *game),
                items,
                occupied,
                options: RenameOptions::default(),
                references: None,
            scripts: None,
                references_revision: None,
                error: None,
                focus: true,
            })
        })();
        match result {
            Ok(dialog) => self.dialogs.open(dialog),
            Err(error) => self.model.status = error,
        }
    }
    pub(in crate::app) fn apply_bulk_rename(&mut self, save: bool, _ctx: &egui::Context) {
        let result = (|| {
            let dialog = self
                .dialogs
                .get::<BulkRenameDialog>()
                .ok_or("Rename dialog closed")?;
            let request = dialog.request.clone();
            let items = dialog.items.clone();
            let options = dialog.options.clone();
            let index = dialog.kit_index(&self.model)
                .ok_or("The workspace changed; select the items again")?;
            if self.tag_ops.folder_refactor.is_some() {
                return Err("A file operation is already running".into());
            }
            if self.model.editing_kit_is_read_only(index) {
                return Err("This editing kit is read-only".into());
            }
            self.focus_kit(index);
            let rows = preview(&items, &options, &occupied_names(&items)?);
            if let Some(error) = rows.iter().find_map(|row| row.error.clone()) {
                return Err(error);
            }
            let roots: Vec<_> = items
                .iter()
                .zip(&rows)
                .filter(|(_, row)| row.changed)
                .map(|(item, row)| (item.path.clone(), row.target.clone()))
                .collect();
            if roots.is_empty() {
                return Err("No names would change".into());
            }
            let dirty: Vec<_> = self.model.kits[index]
                .parsed_tags
                .iter()
                .filter(|(_, doc)| doc.dirty.is_set())
                .map(|(key, _)| key.clone())
                .collect();
            if !dirty.is_empty() && !save {
                return Err("Save unsaved tags before renaming; use Save All and Rename".into());
            }
            for key in dirty {
                self.save_tag_by_key(&key)?;
            }
            let source = self.model.source().ok_or("Workspace closed")?;
            let TagSource::LooseFolder { root, game, .. } = &source.source else {
                return Err("Bulk rename requires a loose editing kit".into());
            };
            let root = root.clone();
            let game = *game;
            let entries = source.full_entry_set().to_vec();
            let names = self.model.names().clone();
            let dependencies = source.reverse_dependencies.clone();
            let context = self.new_merge_context();
            let tx = self.tx.clone();
            let stamp = self.model.kit_stamp();
            let pane = request.pane;
            self.tag_ops.folder_refactor = Some(FolderRefactorUiState {
                label: "Renaming Items".into(),
                phase: "Checking references".into(),
                progress: None,
            });
            self.dialogs.close::<BulkRenameDialog>();
            self.spawn_job(
                move || WorkerMessage::BrowserBulkFinished {
                    stamp,
                    result: folder_merge_ops::run_renames(
                        root,
                        roots,
                        names,
                        game,
                        entries,
                        dependencies,
                        context,
                        &tx,
                    )
                    .map(|done| bulk::BulkOutcome {
                        done,
                        pane,
                        completed: Vec::new(),
                        deleted: Vec::new(),
                        deleted_folders: Vec::new(),
                        renamed: true,
                    }),
                },
                move |_| WorkerMessage::BrowserBulkFinished {
                    stamp,
                    result: Err("Bulk rename worker crashed".into()),
                },
            );
            Ok(())
        })();
        if let Err(error) = result {
            if let Some(dialog) = self.dialogs.get_mut::<BulkRenameDialog>() {
                if let Ok(occupied) = occupied_names(&dialog.items) {
                    dialog.occupied = occupied;
                }
                dialog.error = Some(error);
            } else {
                self.model.status = error;
            }
        }
    }
}

#[cfg(test)]
impl BulkRenameDialog {
    pub(in crate::app) fn for_smoke(stamp: KitStamp) -> Self {
        let items = ["rifle.model", "rifle.render_model"]
            .into_iter()
            .map(|leaf| {
                let path = PathBuf::from("tags").join(leaf);
                RenameItem {
                    stem: path.file_stem().unwrap().to_string_lossy().into_owned(),
                    extension: path.extension().map(std::ffi::OsStr::to_os_string),
                    group_tag: None,
                    path,
                }
            })
            .collect();
        Self {
            request: bulk::BulkRequest { destination: None,
                stamp,
                pane: String::new(),
                keys: Vec::new(),
                action: BulkAction::Rename,
            },
            source_identity: (PathBuf::from("tags"), None),
            items,
            occupied: HashSet::new(),
            options: RenameOptions::default(),
            references: None,
            scripts: None,
            references_revision: None,
            error: None,
            focus: true,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::*;
    /// The referring tags the dialog lists in its count.
    fn known_references(app: &Baboon) -> Vec<String> {
        app.dialogs.get::<BulkRenameDialog>().unwrap().references.as_ref()
            .map(|references| references.paths.iter().map(|(path, _)| path.clone()).collect()).unwrap_or_default()
    }
    fn item(path: PathBuf) -> RenameItem {
        RenameItem {
            stem: path.file_stem().unwrap().to_string_lossy().into_owned(),
            extension: path.extension().map(std::ffi::OsStr::to_os_string),
            group_tag: None,
            path,
        }
    }
    #[test]
    fn shared_stems_keep_extensions_and_parent_folders() {
        let items = vec![
            item("ui/banshee.new_hud_definition".into()),
            item("ui/banshee.weapon".into()),
        ];
        let settings = RenameOptions {
            replacement: "test_{name}_rebalance".into(),
            ..Default::default()
        };
        let rows = preview(&items, &settings, &HashSet::new());
        assert!(rows.iter().all(|row| row.changed && row.error.is_none()));
        assert_eq!(
            rows[0].target,
            PathBuf::from("ui/test_banshee_rebalance.new_hud_definition")
        );
        assert_eq!(
            rows[1].target,
            PathBuf::from("ui/test_banshee_rebalance.weapon")
        );
        let rows = preview(
            &items,
            &RenameOptions {
                replacement: "new_common_name".into(),
                ..Default::default()
            },
            &HashSet::new(),
        );
        assert!(rows.iter().all(|row| row.error.is_none()));
    }
    #[test]
    fn current_names_are_not_reparsed_and_number_tokens_are_rejected() {
        assert_eq!(
            template("prefix_{name}", "literal{n}").unwrap(),
            "prefix_literal{n}"
        );
        assert_eq!(template("{{name}}", "ignored").unwrap(), "{name}");
        for token in ["{n}", "{N}", "{typo}"] {
            assert!(template(token, "name").is_err());
        }
    }

    #[test]
    fn replacement_is_literal_ignores_case_and_leaves_unmatched_names_alone() {
        let items = vec![item("old_old.model".into()), item("other.model".into())];
        let rows = preview(
            &items,
            &RenameOptions {
                find: "old".into(),
                replacement: "new".into(),
                ..Default::default()
            },
            &HashSet::new(),
        );
        assert_eq!(rows[0].target, PathBuf::from("new_new.model"));
        assert!(!rows[1].changed);
        let rows = preview(
            &items,
            &RenameOptions { find: "OLD".into(), replacement: "new".into() },
            &HashSet::new(),
        );
        assert_eq!(rows[0].target, PathBuf::from("new_new.model"), "Find ignores letter case");
        assert_eq!(replace_literal("ÉQUIPE_équipe", "équipe", "team"), "team_team");
        assert_eq!(replace_literal("a.b_a_b", "a.b", "new"), "new_a_b");
        assert_eq!(replace_literal("old", "old", "$1"), "$1");
    }
    #[test]
    fn finding_the_current_name_renames_whole_names_and_empty_find_changes_nothing() {
        let items = vec![item("crate.model".into()), item("Barrel.model".into())];
        let options = |find: &str, replacement: &str| RenameOptions {
            find: find.into(),
            replacement: replacement.into(),
        };
        // The defaults change nothing until Replace With does.
        assert!(preview(&items, &RenameOptions::default(), &HashSet::new()).iter().all(|row| !row.changed && row.error.is_none()));
        let rows = preview(&items, &options("{name}", "old_{name}"), &HashSet::new());
        assert_eq!(rows[0].target, PathBuf::from("old_crate.model"));
        assert_eq!(rows[1].target, PathBuf::from("old_Barrel.model"));
        // {name} in Find is the literal current name, so it can sit inside other text.
        let rows = preview(&items, &options("{name}", "{name}_{name}"), &HashSet::new());
        assert_eq!(rows[0].target, PathBuf::from("crate_crate.model"));
        assert!(preview(&items, &options("", "anything"), &HashSet::new()).iter().all(|row| !row.changed && row.error.is_none()));
        assert!(preview(&items, &options("{typo}", "x"), &HashSet::new()).iter().all(|row| row.error.is_some()));
    }
    #[test]
    fn collisions_and_invalid_names_disable_apply_without_suffixing() {
        let items = vec![item("a.model".into()), item("b.model".into())];
        let rows = preview(
            &items,
            &RenameOptions {
                replacement: "same".into(),
                ..Default::default()
            },
            &HashSet::new(),
        );
        assert!(rows.iter().all(|row| row.error.is_some()));
        for name in ["../escape", "bad.name", "CON", ""] {
            assert!(
                preview(
                    &items,
                    &RenameOptions {
                        replacement: name.into(),
                        ..Default::default()
                    },
                    &HashSet::new()
                )
                .iter()
                .all(|row| row.error.is_some())
            );
        }
        let occupied = HashSet::from([path_key(Path::new("TAKEN.model"))]);
        assert!(
            preview(
                &items[..1],
                &RenameOptions {
                    replacement: "taken".into(),
                    ..Default::default()
                },
                &occupied
            )[0]
            .error
            .is_some()
        );
        assert!(
            preview(
                &items[..1],
                &RenameOptions {
                    replacement: "A".into(),
                    ..Default::default()
                },
                &HashSet::new()
            )[0]
            .error
            .is_some()
        );
    }
    #[test]
    fn reference_preview_includes_all_selected_roots_before_names_change() {
        let kit = LooseKit::new("rename-reference-preview", "halo3_mcc");
        for path in ["group/a", "b"] {
            kit.write_mcc(path, "render_model", |_| {});
        }
        for path in ["group/internal", "outside/shared", "outside/only_b"] {
            kit.write_mcc(path, "model", |_| {});
        }
        let mut source = kit.source();
        assert!(RenameReferences::new(&[], &source).is_none());
        // The preview consumes the reference catalogue, independently of how
        // many fields in a referring tag point at each target.
        let group = group_tag(kit.game, "render_model");
        let dependency = |path: &str| crate::core::source::DependencyRef {
            group_tag: group,
            rel_path: path.into(),
        };
        let mut index = kit.index();
        index.set_tag_dependencies(kit.key("group/internal.model"), [dependency(r"group\a")]);
        index.set_tag_dependencies(
            kit.key("outside/shared.model"),
            [dependency(r"group\a"), dependency("b")],
        );
        index.set_tag_dependencies(kit.key("outside/only_b.model"), [dependency("b")]);
        source.reverse_dependencies = Some(index);
        let items = vec![
            RenameItem {
                path: kit.path("group"),
                stem: "group".into(),
                extension: None,
                group_tag: None,
            },
            item(kit.path("b.render_model")),
        ];
        let references = RenameReferences::new(&items, &source).unwrap();
        assert!(references.complete);
        assert_eq!(references.inside, [2, 0], "the folder's two tags get new paths; a selected tag counts none");
        assert_eq!(folder_tags_note(1, "renamed folders"), "Includes references to 1 tag within renamed folders that will now have a new path.");
        assert_eq!(folder_tags_note(4, "renamed folders"), "Includes references to 4 tags within renamed folders that will now have new paths.");
        assert_eq!(references.paths.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(), [r"group\internal.model", r"outside\only_b.model", r"outside\shared.model"]);
        source
            .reverse_dependencies
            .as_mut()
            .unwrap()
            .clear_tag(&kit.key("outside/only_b.model"));
        assert!(!RenameReferences::new(&items, &source).unwrap().complete);
    }

    fn run(
        kit: &LooseKit,
        roots: Vec<(PathBuf, PathBuf)>,
    ) -> Result<crate::app::shell::FolderRefactorFinished, String> {
        let (tx, _rx) = mpsc::channel();
        let context = folder_merge_ops::MergeContext {
            recovery: Err("Rename must not overwrite anything".into()),
            control: Arc::new(crate::app::tag_ops::folder_merge_review::MergeControl::default()),
        };
        folder_merge_ops::run_renames(
            kit.root.clone(),
            roots,
            kit.names(),
            GameId::from_id(kit.game),
            kit.entries(),
            Some(kit.index()),
            context,
            &tx,
        )
    }
    #[test]
    fn renaming_shared_stems_updates_internal_and_external_references_as_one_batch() {
        let kit = LooseKit::new("bulk-rename-shared-stems", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("objects/crate", "render_model", |_| {});
        kit.write_mcc("objects/crate", "model", |tag| {
            set_reference(tag, "render model", group, "objects\\crate")
        });
        kit.write_mcc("other/uses_crate", "model", |tag| {
            set_reference(tag, "render model", group, "objects\\crate")
        });
        let roots = vec![
            (
                kit.path("objects/crate.model"),
                kit.path("objects/new_crate.model"),
            ),
            (
                kit.path("objects/crate.render_model"),
                kit.path("objects/new_crate.render_model"),
            ),
        ];
        let done = run(&kit, roots).unwrap();
        assert_eq!(done.old_to_new_keys.len(), 2);
        assert!(done.status.starts_with("Renamed"));
        assert!(!kit.path("objects/crate.model").exists());
        for name in ["objects/new_crate.model", "other/uses_crate.model"] {
            assert_eq!(
                reference_of(&TagFile::read(kit.path(name)).unwrap(), "render model").as_deref(),
                Some("objects\\new_crate")
            );
        }
    }
    #[test]
    fn a_rename_collision_never_merges_folders_or_overwrites_files() {
        let kit = LooseKit::new("bulk-rename-collision", "halo3_mcc");
        kit.write_mcc("original/a", "model", |_| {});
        kit.write_mcc("existing/b", "model", |_| {});
        let before = std::fs::read(kit.path("original/a.model")).unwrap();
        let error = run(&kit, vec![(kit.path("original"), kit.path("existing"))])
            .err()
            .unwrap();
        assert!(error.contains("already exists"));
        assert_eq!(std::fs::read(kit.path("original/a.model")).unwrap(), before);
        assert!(!kit.path("existing/a.model").exists());
    }
    #[test]
    fn dialog_preview_is_live_and_cancel_changes_nothing() {
        let kit = LooseKit::new("bulk-rename-dialog", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("crate", "model", |tag| set_reference(tag, "render model", group, "crate"));
        kit.write_mcc("other/uses_crate", "model", |tag| set_reference(tag, "render model", group, "crate"));
        kit.write_mcc("crate", "render_model", |_| {});
        let mut app = app();
        kit.install_indexed(&mut app);
        let keys = vec![kit.key("crate.model"), kit.key("crate.render_model")];
        app.begin_browser_bulk("pane".into(), keys, BulkAction::Rename, &ctx());
        let mut driver = PromptDriver::new();
        driver.frame(&mut app, Vec::new());
        let initial = driver.frame(&mut app, Vec::new());
        assert!(initial.iter().any(|(text, _)| text == "2 References to Fix"));
        app.dialogs
            .get_mut::<BulkRenameDialog>()
            .unwrap()
            .options
            .replacement = "prefix_{name}_suffix".into();
        driver.frame(&mut app, Vec::new());
        let page = driver.frame(&mut app, Vec::new());
        for label in [
            "Bulk Rename",
            "Original",
            "Rename",
            "2 References to Fix",
            "prefix_crate_suffix.model",
            "prefix_crate_suffix.render_model",
            "2 items selected · renaming 2 tags",
        ] {
            assert!(
                page.iter().any(|(text, _)| text == label),
                "missing {label}: {page:?}"
            );
        }
        assert!(!page.iter().any(|(text, _)| matches!(text.as_str(),
            "Number ↑" | "Number ↓" | "Start Ascending At" | "Digits")));
        assert!(known_references(&app).iter().any(|path| path == r"other\uses_crate.model"));
        driver.click(&mut app, "Cancel");
        assert!(app.dialogs.get::<BulkRenameDialog>().is_none());
        assert!(kit.path("crate.model").exists());
        assert!(!kit.path("prefix_crate_suffix.model").exists());
    }
    #[test]
    fn about_opens_renaming_help_above_the_locked_rename_window() {
        let kit = LooseKit::new("bulk-rename-about", "halo3_mcc");
        kit.write_mcc("crate", "model", |_| {});
        let mut app = app();
        kit.install_indexed(&mut app);
        app.begin_browser_bulk("pane".into(), vec![kit.key("crate.model")], BulkAction::Rename, &ctx());
        let mut driver = PromptDriver::new();
        driver.frame(&mut app, Vec::new());
        // What the header's About button sends.
        app.apply_help_command(crate::app::help::HelpCommand::ShowDocSection(crate::app::help::RENAMING_HELP_SECTION));
        driver.frame(&mut app, Vec::new());
        let page = driver.frame(&mut app, Vec::new());
        let help = app.dialogs.get::<crate::app::help::HelpWindow>().unwrap();
        assert_eq!(help.doc_section.as_deref(), Some("Renaming"));
        assert!(help.above_dialogs);
        assert!(page.iter().any(|(text, _)| text.contains("Find and Replace match names only")), "Renaming page not shown: {page:?}");
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("baboon_help"));
        assert!(driver.ctx.memory(|memory| memory.allows_interaction(layer)), "Help is locked under the rename window");
        assert!(app.dialogs.get::<BulkRenameDialog>().is_some(), "opening About keeps Bulk Rename open");
    }
    #[test]
    fn refresh_tolerance_does_not_allow_a_different_editing_kit_or_engine() {
        let original = LooseKit::new("rename-source-guard", "halo3_mcc");
        let replacement = LooseKit::new("rename-source-replacement", "halo3_mcc");
        original.write_mcc("crate", "model", |_| {});
        replacement.write_mcc("crate", "model", |_| {});
        let mut app = app();
        original.install_indexed(&mut app);
        app.begin_browser_bulk(
            "pane".into(),
            vec![original.key("crate.model")],
            BulkAction::Rename,
            &ctx(),
        );
        app.dialogs
            .get_mut::<BulkRenameDialog>()
            .unwrap()
            .options
            .replacement = "new_{name}".into();
        app.model.kits[0].source = Some(replacement.source());
        app.model.kits[0].generation += 1;
        app.apply_bulk_rename(false, &ctx());
        assert!(app.tag_ops.folder_refactor.is_none());
        assert!(
            app.dialogs
                .get::<BulkRenameDialog>()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("workspace changed")
        );
        // The same path reopened with another engine is a different source too.
        let mut source = original.source();
        if let TagSource::LooseFolder { game, .. } = &mut source.source {
            *game = GameId::from_id("halo2_mcc");
        }
        app.model.kits[0].source = Some(source);
        app.apply_bulk_rename(false, &ctx());
        assert!(app.tag_ops.folder_refactor.is_none());
        for kit in [&original, &replacement] {
            assert!(kit.path("crate.model").exists());
            assert!(!kit.path("new_crate.model").exists());
        }
    }

    #[test]
    fn index_refresh_keeps_the_reference_list_and_allows_rename_afterward() {
        let kit = LooseKit::new("rename-during-index-refresh", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("crate", "render_model", |_| {});
        kit.write_mcc("crate", "model", |tag| {
            set_reference(tag, "render model", group, "crate")
        });
        kit.write_mcc("a_user", "model", |tag| {
            set_reference(tag, "render model", group, "crate")
        });
        kit.write_mcc("b_user", "model", |_| {});
        let mut app = app();
        kit.install_indexed(&mut app);
        let ctx = ctx();
        app.begin_browser_bulk(
            "pane".into(),
            vec![kit.key("crate.model"), kit.key("crate.render_model")],
            BulkAction::Rename,
            &ctx,
        );
        app.dialogs
            .get_mut::<BulkRenameDialog>()
            .unwrap()
            .options
            .replacement = "new_{name}".into();
        let mut driver = PromptDriver::new();
        driver.frame(&mut app, Vec::new());
        driver.frame(&mut app, Vec::new());
        assert!(known_references(&app).iter().any(|path| path == "a_user.model"));
        // A real periodic refresh patches references and advances generation,
        // even when the entry and reference counts remain exactly the same.
        kit.write_mcc("a_user", "model", |_| {});
        kit.write_mcc("b_user", "model", |tag| {
            set_reference(tag, "render model", group, "crate")
        });
        let refreshed = kit.index();
        let entries = kit.entries();
        let touched: Vec<_> = entries
            .iter()
            .filter(|entry| {
                entry.display_path == "a_user.model" || entry.display_path == "b_user.model"
            })
            .cloned()
            .collect();
        let dependencies = touched
            .iter()
            .map(|entry| {
                (
                    entry.key.clone(),
                    refreshed.dependencies_of(&entry.key).to_vec(),
                )
            })
            .collect();
        let stamp = app.model.kit_stamp();
        app.handle_entry_index_refreshed(
            stamp,
            Ok(crate::core::source::EntryIndexRefresh {
                entries,
                changed: true,
                added: 0,
                updated: 2,
                removed: 0,
                touched,
                removed_keys: Vec::new(),
                touched_dependencies: dependencies,
                errors: Vec::new(),
            }),
            &ctx,
        );
        let page = driver.frame(&mut app, Vec::new());
        assert!(
            page.iter()
                .any(|(text, _)| text == "2 References to Fix"),
            "reference list vanished after refresh: {page:?}"
        );
        assert!(known_references(&app).iter().any(|path| path == "b_user.model"));
        assert!(!known_references(&app).iter().any(|path| path == "a_user.model"));
        assert_eq!(
            app.dialogs
                .get::<BulkRenameDialog>()
                .unwrap()
                .options
                .replacement,
            "new_{name}"
        );
        // While a rebuild is in flight, retain the last known list.
        app.model.kits[0]
            .source
            .as_mut()
            .unwrap()
            .reverse_dependencies = None;
        let page = driver.frame(&mut app, Vec::new());
        assert!(page.iter().any(|(text, _)| text == "2 References to Fix"));
        assert!(known_references(&app).iter().any(|path| path == "b_user.model"));
        let stamp = app.model.kit_stamp();
        app.handle_reverse_dependencies_built(stamp, refreshed, 0);
        driver.frame(&mut app, Vec::new());
        app.apply_bulk_rename(false, &ctx);
        assert!(
            app.tag_ops.folder_refactor.is_some(),
            "{}",
            app.model.status
        );
        pump_until(&mut app, "rename after index refresh", |app| {
            app.tag_ops.folder_refactor.is_none()
        });
        assert!(kit.path("new_crate.render_model").exists());
        assert_eq!(
            reference_of(
                &TagFile::read(kit.path("b_user.model")).unwrap(),
                "render model"
            )
            .as_deref(),
            Some("new_crate")
        );
    }

    #[test]
    fn mixed_folder_and_tag_rename_preserves_contents_and_rewrites_referrers() {
        let kit = LooseKit::new("rename-mixed-roots", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("group/part", "render_model", |_| {});
        kit.write_mcc("loose", "render_model", |_| {});
        kit.write_mcc("outside/uses_group", "model", |tag| {
            set_reference(tag, "render model", group, "group\\part")
        });
        // A reference between two tags inside the renamed folder.
        kit.write_mcc("group/inner/uses_part", "model", |tag| {
            set_reference(tag, "render model", group, "group\\part")
        });
        fs::create_dir_all(kit.path("group/empty")).unwrap();
        fs::write(kit.path("group/note.txt"), "note").unwrap();
        let done = run(
            &kit,
            vec![
                (kit.path("group"), kit.path("new_group")),
                (
                    kit.path("loose.render_model"),
                    kit.path("new_loose.render_model"),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            done.moved_folders,
            vec![(PathBuf::from("group"), PathBuf::from("new_group"))]
        );
        assert!(kit.path("new_group/empty").is_dir());
        assert_eq!(
            fs::read_to_string(kit.path("new_group/note.txt")).unwrap(),
            "note"
        );
        assert_eq!(
            reference_of(
                &TagFile::read(kit.path("outside/uses_group.model")).unwrap(),
                "render model"
            )
            .as_deref(),
            Some("new_group\\part")
        );
        assert_eq!(
            reference_of(
                &TagFile::read(kit.path("new_group/inner/uses_part.model")).unwrap(),
                "render model"
            )
            .as_deref(),
            Some("new_group\\part"),
            "a reference inside the renamed folder follows it too"
        );
    }
    #[test]
    fn rename_completion_keeps_the_new_names_selected_and_remaps_open_tabs() {
        let kit = LooseKit::new("rename-completion", "halo3_mcc");
        kit.write_mcc("crate", "model", |_| {});
        let mut app = app();
        kit.install(&mut app);
        let ctx = ctx();
        app.handle_browser_action(
            BrowserAction::OpenFolderBrowser {
                rel_path: PathBuf::new(),
                label: "Tags".into(),
                open_in_new_tab: true,
            },
            ctx.clone(),
        );
        let pane = app.model.kits[0].current_pane.clone().unwrap();
        let old = kit.key("crate.model");
        let new = crate::core::source::file_entry_key(&kit.path("new_crate.model"));
        let id = app.model.active_kit_id();
        let browser = app.views[id]
            .browser
            .folder_browsers
            .get_mut(&pane)
            .unwrap();
        browser.selection.keys.insert(old.clone());
        browser.selection.focus = Some(old.clone());
        browser.selected_tag = Some(old.clone());
        app.kit_and_view(0).open_tag_pane(&old);
        let stamp = app.model.kit_stamp();
        let done = run(
            &kit,
            vec![(kit.path("crate.model"), kit.path("new_crate.model"))],
        )
        .unwrap();
        app.handle_browser_bulk_finished(
            stamp,
            Ok(bulk::BulkOutcome {
                done,
                pane: pane.clone(),
                completed: Vec::new(),
                deleted: Vec::new(),
                deleted_folders: Vec::new(),
                renamed: true,
            }),
            &ctx,
        );
        assert!(app.model.kits[0].open_tabs.contains(&new));
        assert!(!app.model.kits[0].open_tabs.contains(&old));
        assert_eq!(
            app.views[id].browser.folder_browsers[&pane].selection.keys,
            std::collections::BTreeSet::from([new])
        );
    }
}
