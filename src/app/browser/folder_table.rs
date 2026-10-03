//! Explorer columns and sibling sorting, scoped to docked folder pages.
use super::*;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum FolderColumn {
    Name,
    Modified,
    Size,
    Keywords,
}
impl FolderColumn {
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Modified => "Date Modified",
            Self::Size => "Size",
            Self::Keywords => "Keywords",
        }
    }
    fn minimum(self) -> f32 {
        if self == Self::Name { 160.0 } else { 90.0 }
    }
}

#[derive(Clone)]
pub(in crate::app) struct FolderTableLayout {
    columns: Vec<(FolderColumn, f32)>,
    sort: Option<(FolderColumn, bool)>, // true = descending
    initialized: bool,
}
impl Default for FolderTableLayout {
    fn default() -> Self {
        Self {
            columns: vec![
                (FolderColumn::Name, 380.0),
                (FolderColumn::Modified, 200.0),
                (FolderColumn::Size, 110.0),
                (FolderColumn::Keywords, 240.0),
            ],
            sort: None,
            initialized: false,
        }
    }
}
impl FolderTableLayout {
    pub(in crate::app) fn has_sort(&self) -> bool {
        self.sort.is_some()
    }
    pub(in crate::app) fn clear_sort(&mut self) {
        self.sort = None;
    }
    pub(in crate::app) fn width(&self) -> f32 {
        self.columns.iter().map(|(_, width)| width).sum()
    }
    fn toggle_sort(&mut self, column: FolderColumn) {
        self.sort = Some((
            column,
            self.sort
                .is_some_and(|(active, descending)| active == column && !descending),
        ));
    }
    fn reorder(&mut self, source: FolderColumn, target: FolderColumn) {
        if source == FolderColumn::Name || target == FolderColumn::Name || source == target {
            return;
        }
        let from = self
            .columns
            .iter()
            .position(|(column, _)| *column == source)
            .unwrap();
        let to = self
            .columns
            .iter()
            .position(|(column, _)| *column == target)
            .unwrap();
        let column = self.columns.remove(from);
        self.columns.insert(to, column);
    }
    fn resize(&mut self, index: usize, delta: f32) {
        let (column, width) = &mut self.columns[index];
        *width = (*width + delta).max(column.minimum());
    }
}

#[derive(Clone, Default)]
struct FileDetails {
    modified: Option<SystemTime>,
    size: Option<u64>,
    date: String,
}
#[derive(Clone, Default)]
pub(in crate::app) struct FolderDateCache(Arc<Mutex<HashMap<PathBuf, (Instant, FileDetails)>>>);
impl FolderDateCache {
    fn details(&self, path: &Path) -> FileDetails {
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((at, details)) = cache.get(path)
            && at.elapsed() < Duration::from_secs(2)
        {
            return details.clone();
        }
        let metadata = std::fs::metadata(path).ok();
        let modified = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok());
        let details = FileDetails {
            modified,
            size: metadata
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len()),
            date: modified.and_then(local_date).unwrap_or_else(|| "—".into()),
        };
        cache.insert(path.to_owned(), (Instant::now(), details.clone()));
        details
    }
    #[cfg(test)]
    fn date(&self, path: &Path) -> String {
        self.details(path).date
    }
}

#[derive(Clone)]
struct FolderTable {
    columns: Vec<(FolderColumn, egui::Rect)>,
    sort: Option<(FolderColumn, bool)>,
    root: Option<PathBuf>,
    keywords: Arc<BTreeMap<String, Vec<String>>>,
    dates: FolderDateCache,
}
fn table_id() -> egui::Id {
    egui::Id::new("folder_browser_table_columns")
}

pub(in crate::app) fn begin_folder_table(
    ui: &mut Ui,
    root: Option<PathBuf>,
    keywords: Arc<BTreeMap<String, Vec<String>>>,
    dates: FolderDateCache,
    layout: &mut FolderTableLayout,
) {
    if !layout.initialized {
        let scale = (ui.available_width() / layout.width()).max(1.0);
        for (_, width) in &mut layout.columns {
            *width *= scale;
        }
        layout.initialized = true;
    }
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(
            ui.available_width().max(layout.width()),
            ui.spacing().interact_size.y + 6.0,
        ),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0_f32, foundation_input_edge()));
    let mut x = rect.left();
    let mut columns = Vec::new();
    let mut reorder = None;
    let mut clicked = None;
    // Sizes and order may change while interacting; the next repaint applies them together.
    for (index, (column, width)) in layout.columns.clone().into_iter().enumerate() {
        let cell = egui::Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(x + width, rect.bottom()),
        );
        columns.push((column, cell));
        let header = ui.interact(
            cell.shrink2(Vec2::new(5.0, 0.0)),
            ui.make_persistent_id(("folder_column", column)),
            if column == FolderColumn::Name {
                Sense::click()
            } else {
                Sense::click_and_drag()
            },
        );
        if header.clicked() {
            clicked = Some(column);
        }
        if header.drag_started() {
            header.dnd_set_drag_payload(column);
        }
        if column != FolderColumn::Name {
            if header.dnd_hover_payload::<FolderColumn>().is_some() {
                ui.painter().rect_stroke(
                    cell.shrink(2.0),
                    0.0,
                    Stroke::new(1.0_f32, ui.visuals().selection.stroke.color),
                );
            }
            if let Some(source) = header.dnd_release_payload::<FolderColumn>() {
                reorder = Some((*source, column));
            }
        }
        let arrow = match layout.sort {
            Some((active, descending)) if active == column => {
                if descending {
                    " ▼"
                } else {
                    " ▲"
                }
            }
            _ => "",
        };
        let label = format!("{}{arrow}", column.label());
        let text_offset = if column == FolderColumn::Name {
            ui.spacing().indent + ui.spacing().item_spacing.x + 21.0
        } else {
            8.0
        };
        ui.painter()
            .with_clip_rect(ui.clip_rect().intersect(cell.shrink2(Vec2::new(8.0, 0.0))))
            .text(
                cell.left_center() + Vec2::new(text_offset, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(13.0),
                text_dark(),
            );
        let divider = egui::Rect::from_min_max(
            egui::pos2(cell.right() - 4.0, rect.top()),
            egui::pos2(cell.right() + 4.0, rect.bottom()),
        );
        let resize = ui
            .interact(
                divider,
                ui.make_persistent_id(("folder_column_resize", column)),
                Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        // A drag-only handle activates on mouse-down. Its first pointer delta
        // can include travel to the handle before pressing, which is not a resize.
        if resize.dragged() && !ui.input(|input| input.pointer.any_pressed()) {
            layout.resize(index, ui.input(|input| input.pointer.delta().x));
            ui.ctx().request_repaint();
        }
        ui.painter().line_segment(
            [
                egui::pos2(cell.right(), rect.top()),
                egui::pos2(cell.right(), rect.bottom()),
            ],
            Stroke::new(1.0_f32, foundation_input_edge()),
        );
        x += width;
    }
    if let Some(column) = clicked {
        layout.toggle_sort(column);
    }
    if let Some((source, target)) = reorder {
        layout.reorder(source, target);
        ui.ctx().request_repaint();
    }
    ui.data_mut(|data| {
        data.insert_temp(
            table_id(),
            FolderTable {
                columns,
                sort: layout.sort,
                root,
                keywords,
                dates,
            },
        )
    });
}
pub(in crate::app) fn end_folder_table(ui: &Ui) {
    ui.data_mut(|data| data.remove::<FolderTable>(table_id()));
}
pub(in crate::app) fn folder_name_clip(ui: &Ui) -> egui::Rect {
    let mut clip = ui.clip_rect();
    if let Some(table) = ui.data(|data| data.get_temp::<FolderTable>(table_id())) {
        clip.max.x = clip.max.x.min(table.columns[0].1.right() - 8.0);
    }
    clip
}
pub(in crate::app) fn folder_table_active(ui: &Ui) -> bool {
    ui.data(|data| data.get_temp::<FolderTable>(table_id()).is_some())
}
pub(in crate::app) fn paint_folder_columns(
    ui: &Ui,
    rect: egui::Rect,
    entry: Option<&TagEntry>,
    folder: Option<&Path>,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let Some(table) = ui.data(|data| data.get_temp::<FolderTable>(table_id())) else {
        return;
    };
    let path = entry
        .and_then(entry_loose_file)
        .or_else(|| folder.and_then(|folder| table.root.as_ref().map(|root| root.join(folder))));
    let details = path.as_deref().map(|path| table.dates.details(path));
    for (column, header) in table.columns.iter().skip(1) {
        let cell = egui::Rect::from_min_max(
            egui::pos2(header.left() + 8.0, rect.top()),
            egui::pos2(header.right() - 8.0, rect.bottom()),
        );
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(cell));
        if *column == FolderColumn::Keywords {
            let Some(keywords) = entry.and_then(|entry| table.keywords.get(&entry.key)) else {
                continue;
            };
            let mut x = cell.left();
            for keyword in keywords {
                let galley = painter.layout_no_wrap(
                    keyword.clone(),
                    FontId::proportional(12.0),
                    text_dark(),
                );
                let pill = egui::Rect::from_min_size(
                    egui::pos2(x, rect.center().y - 9.0),
                    Vec2::new(galley.size().x + 14.0, 18.0),
                );
                painter.rect_filled(pill, 9.0, ui.visuals().widgets.inactive.weak_bg_fill);
                painter.galley(
                    egui::pos2(x + 7.0, rect.center().y - galley.size().y * 0.5),
                    galley,
                    text_dark(),
                );
                x = pill.right() + 4.0;
            }
            ui.interact(
                cell,
                ui.make_persistent_id(("folder_keywords", &entry.unwrap().key)),
                Sense::hover(),
            )
            .on_hover_text(keywords.join(", "));
        } else {
            let text = match column {
                FolderColumn::Modified => details
                    .as_ref()
                    .map(|details| details.date.clone())
                    .unwrap_or_else(|| "—".into()),
                FolderColumn::Size if folder.is_some() => String::new(),
                FolderColumn::Size => details
                    .as_ref()
                    .and_then(|details| details.size)
                    .map(format_file_size)
                    .unwrap_or_else(|| "—".into()),
                _ => unreachable!(),
            };
            painter.text(
                cell.left_center(),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(12.5),
                text_dark(),
            );
        }
    }
}
fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB"] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SortValue {
    Name(String),
    Date(SystemTime),
    Size(u64),
    Keywords(Vec<String>),
}
impl FolderTable {
    fn value(
        &self,
        column: FolderColumn,
        name: &str,
        path: Option<&Path>,
        key: Option<&str>,
    ) -> Option<SortValue> {
        match column {
            FolderColumn::Name => Some(SortValue::Name(name.to_ascii_lowercase())),
            FolderColumn::Modified => path
                .and_then(|path| self.dates.details(path).modified)
                .map(SortValue::Date),
            FolderColumn::Size => path
                .and_then(|path| self.dates.details(path).size)
                .map(SortValue::Size),
            FolderColumn::Keywords => key
                .and_then(|key| self.keywords.get(key))
                .filter(|list| !list.is_empty())
                .cloned()
                .map(SortValue::Keywords),
        }
    }
}
fn compare_values(a: &Option<SortValue>, b: &Option<SortValue>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if descending {
                b.cmp(a)
            } else {
                a.cmp(b)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
pub(in crate::app) fn table_order_entries(
    ui: &Ui,
    indices: &[usize],
    entries: &[TagEntry],
) -> Option<Vec<usize>> {
    let table = ui.data(|data| data.get_temp::<FolderTable>(table_id()))?;
    let (column, descending) = table.sort?;
    let mut rows: Vec<_> = indices
        .iter()
        .map(|&index| {
            let entry = &entries[index];
            let name = entry
                .display_path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&entry.display_path)
                .to_ascii_lowercase();
            let path = entry_loose_file(entry);
            (
                index,
                table.value(column, &name, path.as_deref(), Some(&entry.key)),
                name,
            )
        })
        .collect();
    rows.sort_by(|a, b| {
        compare_values(&a.1, &b.1, descending)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    Some(rows.into_iter().map(|row| row.0).collect())
}
pub(in crate::app) fn table_order_folders(ui: &Ui, nodes: &[TagTreeNode]) -> Option<Vec<usize>> {
    let table = ui.data(|data| data.get_temp::<FolderTable>(table_id()))?;
    let (column, descending) = table.sort?;
    let mut rows: Vec<_> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let name = node.label.to_ascii_lowercase();
            let path = table.root.as_ref().map(|root| root.join(&node.rel_path));
            (
                index,
                table.value(column, &name, path.as_deref(), None),
                name,
            )
        })
        .collect();
    rows.sort_by(|a, b| {
        compare_values(&a.1, &b.1, descending)
            .then_with(|| {
                if column == FolderColumn::Name && descending {
                    b.2.cmp(&a.2)
                } else {
                    a.2.cmp(&b.2)
                }
            })
            .then_with(|| a.0.cmp(&b.0))
    });
    Some(rows.into_iter().map(|row| row.0).collect())
}

fn local_date(time: SystemTime) -> Option<String> {
    let seconds: libc::time_t = time
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs()
        .try_into()
        .ok()?;
    let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
    // Both APIs write into caller-owned storage, avoiding localtime's shared static buffer.
    #[cfg(windows)]
    let success = unsafe { libc::localtime_s(calendar.as_mut_ptr(), &seconds) == 0 };
    #[cfg(not(windows))]
    let success = unsafe { !libc::localtime_r(&seconds, calendar.as_mut_ptr()).is_null() };
    if !success {
        return None;
    }
    let calendar = unsafe { calendar.assume_init() };
    Some(format!(
        "{:04}-{:02}-{:02}  {:02}:{:02}",
        calendar.tm_year + 1900,
        calendar.tm_mon + 1,
        calendar.tm_mday,
        calendar.tm_hour,
        calendar.tm_min
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_click_resize_and_drag_reorder_work_and_name_stays_first() {
        let ctx = egui::Context::default();
        let mut layout = FolderTableLayout::default();
        let frame = |layout: &mut FolderTableLayout, events: Vec<egui::Event>| {
            let mut columns = Vec::new();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1100.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        begin_folder_table(
                            ui,
                            None,
                            Arc::new(BTreeMap::new()),
                            FolderDateCache::default(),
                            layout,
                        );
                        columns = ui
                            .data(|data| data.get_temp::<FolderTable>(table_id()).unwrap().columns);
                        end_folder_table(ui);
                    });
                },
            );
            columns
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let columns = frame(&mut layout, vec![]);
        let date_center = columns[1].1.center();
        for descending in [false, true, false] {
            frame(
                &mut layout,
                vec![
                    egui::Event::PointerMoved(date_center),
                    button(date_center, true),
                ],
            );
            frame(&mut layout, vec![button(date_center, false)]);
            assert_eq!(layout.sort, Some((FolderColumn::Modified, descending)));
        }
        let divider = columns[0].1.right_center();
        let before = layout.columns[0].1;
        frame(
            &mut layout,
            vec![egui::Event::PointerMoved(divider), button(divider, true)],
        );
        let moved = divider + Vec2::new(40.0, 0.0);
        frame(&mut layout, vec![egui::Event::PointerMoved(moved)]);
        frame(&mut layout, vec![button(moved, false)]);
        assert!(
            (layout.columns[0].1 - before - 40.0).abs() < 0.1,
            "before={before}, after={}",
            layout.columns[0].1
        );
        let columns = frame(&mut layout, vec![]);
        let source = columns[3].1.center();
        let target = columns[1].1.center();
        frame(
            &mut layout,
            vec![egui::Event::PointerMoved(source), button(source, true)],
        );
        frame(
            &mut layout,
            vec![egui::Event::PointerMoved(source + Vec2::new(-20.0, 0.0))],
        );
        frame(&mut layout, vec![egui::Event::PointerMoved(target)]);
        frame(&mut layout, vec![button(target, false)]);
        assert_eq!(layout.columns[1].0, FolderColumn::Keywords);
        layout.reorder(FolderColumn::Name, FolderColumn::Size);
        layout.reorder(FolderColumn::Size, FolderColumn::Name);
        assert_eq!(layout.columns[0].0, FolderColumn::Name);
        layout.resize(0, -10_000.0);
        assert_eq!(layout.columns[0].1, FolderColumn::Name.minimum());
    }

    #[test]
    fn sibling_sort_uses_numeric_sizes_dates_and_keywords_with_unknowns_last() {
        let ctx = egui::Context::default();
        let entries: Vec<_> = ["z", "a", "missing"]
            .into_iter()
            .map(|name| TagEntry {
                key: name.into(),
                display_path: format!("{name}.bitmap"),
                group_tag: u32::from_be_bytes(*b"bitm"),
                group_name: None,
                location: TagEntryLocation::LooseFile(PathBuf::from(name)),
            })
            .collect();
        let cache = FolderDateCache::default();
        for (name, size, seconds) in [("z", 9, 10), ("a", 100, 20)] {
            cache.0.lock().unwrap().insert(
                name.into(),
                (
                    Instant::now(),
                    FileDetails {
                        modified: Some(UNIX_EPOCH + Duration::from_secs(seconds)),
                        size: Some(size),
                        date: String::new(),
                    },
                ),
            );
        }
        let keywords = Arc::new(BTreeMap::from([
            ("z".into(), vec!["alpha".into()]),
            ("a".into(), vec!["wip".into()]),
        ]));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut layout = FolderTableLayout::default();
                for column in [
                    FolderColumn::Size,
                    FolderColumn::Modified,
                    FolderColumn::Keywords,
                ] {
                    layout.sort = Some((column, false));
                    begin_folder_table(ui, None, Arc::clone(&keywords), cache.clone(), &mut layout);
                    assert_eq!(
                        table_order_entries(ui, &[0, 1, 2], &entries),
                        Some(vec![0, 1, 2])
                    );
                    end_folder_table(ui);
                    layout.toggle_sort(column);
                    begin_folder_table(ui, None, Arc::clone(&keywords), cache.clone(), &mut layout);
                    assert_eq!(
                        table_order_entries(ui, &[0, 1, 2], &entries),
                        Some(vec![1, 0, 2])
                    );
                    end_folder_table(ui);
                }
                // Sorting receives one sibling list: entries in other branches remain untouched.
                layout.sort = Some((FolderColumn::Name, true));
                begin_folder_table(ui, None, Arc::clone(&keywords), cache.clone(), &mut layout);
                assert_eq!(table_order_entries(ui, &[0, 1], &entries), Some(vec![0, 1]));
                end_folder_table(ui);
                assert_eq!(table_order_entries(ui, &[0, 1], &entries), None);
            });
        });
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(1024), "1.0 KiB");
        assert_eq!(format_file_size(1_048_576), "1.0 MiB");
    }

    #[test]
    fn table_columns_align_across_tree_depths_and_do_not_leak_into_sidebar() {
        let ctx = egui::Context::default();
        let entry = TagEntry {
            key: "example".into(),
            display_path: "objects/example.bitmap".into(),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: None,
            location: TagEntryLocation::Monolithic {
                name: "example".into(),
                group_tag: u32::from_be_bytes(*b"bitm"),
            },
        };
        let mut keyword_map = BTreeMap::new();
        keyword_map.insert(entry.key.clone(), vec!["wip".into(), "needs work".into()]);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(900.0, 500.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let sidebar_clip = ui.clip_rect();
                    assert_eq!(folder_name_clip(ui), sidebar_clip);
                    begin_folder_table(
                        ui,
                        None,
                        Arc::new(keyword_map.clone()),
                        FolderDateCache::default(),
                        &mut FolderTableLayout::default(),
                    );
                    assert!(folder_name_clip(ui).right() < sidebar_clip.right());
                    draw_entry(ui, &entry, None, false, true, None, None, true);
                    ui.indent("nested", |ui| {
                        draw_entry(ui, &entry, None, false, true, None, None, true);
                    });
                    end_folder_table(ui);
                    assert_eq!(folder_name_clip(ui), sidebar_clip);
                    draw_entry(ui, &entry, None, false, true, None, None, true);
                });
            },
        );
        let text_positions = |text: &str| -> Vec<f32> {
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(shape) if shape.galley.job.text == text => Some(shape.pos.x),
                    _ => None,
                })
                .collect()
        };
        for heading in ["Name", "Date Modified", "Size", "Keywords"] {
            assert_eq!(text_positions(heading).len(), 1);
        }
        assert_eq!(
            text_positions("Name")[0],
            text_positions("example.bitmap")[0]
        );
        for text in ["wip", "needs work"] {
            let positions = text_positions(text);
            assert_eq!(
                positions.len(),
                2,
                "Only the two folder table rows display {text}"
            );
            assert_eq!(positions[0], positions[1], "Nested rows must align {text}");
        }
    }

    #[test]
    fn dates_use_filesystem_metadata_and_missing_files_have_no_date() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let expected = local_date(std::fs::metadata(&path).unwrap().modified().unwrap()).unwrap();
        let cache = FolderDateCache::default();
        assert_eq!(cache.date(&path), expected);
        assert_eq!(cache.date(&path.with_extension("definitely-missing")), "—");
    }
}
