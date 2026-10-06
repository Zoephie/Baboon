//! The thumbnail libraries: a kit's tags of one kind as a searchable grid.
//! It owns what the Bitmap Library and the Model Library share: their state,
//! the bounded thumbnail cache, the cell metrics and grid arithmetic, the
//! toolbar, the cells, and the worker queue. What differs between them — which
//! tags are listed, how a thumbnail is made, and what a cell's menu offers — is
//! a [`ThumbnailSource`], implemented in `bitmap_browser` and `model_browser`.

use super::*;
use std::marker::PhantomData;

/// How many decoded thumbnails are held at once.
///
/// Each is a GPU texture, so this cannot be the unbounded map the tag editor's
/// `bitmap_previews` is: that one holds a full-resolution RGBA buffer plus a
/// texture per bitmap the user opened, which is fine for a handful of tabs and
/// ruinous across a kit with thousands of bitmaps. Dropping the handle frees
/// the texture.
pub(in crate::app) const THUMBNAIL_CACHE_CAP: usize = 512;

/// How many are dropped once the cap is passed.
///
/// A batch, rather than one per insert: eviction scans the cache for the least
/// recently drawn entries, and doing that on every new thumbnail while the user
/// scrolls would be the most expensive thing in the frame.
pub(in crate::app) const THUMBNAIL_EVICT_BATCH: usize = 128;

/// Decode jobs allowed to run at once.
///
/// A fast scroll can want a hundred new thumbnails in a frame; without a bound
/// that is a hundred threads, all reading tags off the same disk. Four keeps
/// the queue moving without the frame ever waiting on it.
pub(in crate::app) const MAX_DECODES_IN_FLIGHT: usize = 4;

pub(in crate::app) const MIN_CELL: f32 = 48.0;
pub(in crate::app) const MAX_CELL: f32 = 224.0;
pub(in crate::app) const DEFAULT_CELL: f32 = 96.0;

/// Room under the image for the name and the group.
pub(in crate::app) const CELL_CAPTION: f32 = 30.0;
pub(in crate::app) const CELL_GAP: f32 = 8.0;

/// A bounded, least-recently-drawn thumbnail cache.
///
/// `None` is cached as well as `Some`: a bitmap that fails to decode — an empty
/// tag, an unsupported format — must not be retried on every frame it is
/// visible, which is what made the shader editor's inline thumbnails cache
/// their failures too.
#[derive(Default)]
pub(in crate::app) struct ThumbnailCache {
    pub(in crate::app) entries: HashMap<String, Thumbnail>,
    /// Monotonic draw counter; `Thumbnail::used` is a stamp from it.
    clock: u64,
}

pub(in crate::app) struct Thumbnail {
    texture: Option<egui::TextureHandle>,
    used: u64,
    /// The loose file's modified time when this was decoded; `None` for tags
    /// that live in a pak or cache, which do not change underneath.
    modified: Option<std::time::SystemTime>,
}

/// A loose tag's modified time, from its `file:` key.
fn loose_tag_modified(key: &str) -> Option<std::time::SystemTime> {
    let path = file_key_path(key)?;
    std::fs::metadata(path).ok()?.modified().ok()
}

impl ThumbnailCache {
    pub(in crate::app) fn get(&mut self, key: &str) -> Option<Option<egui::TextureHandle>> {
        self.clock += 1;
        let clock = self.clock;
        let thumbnail = self.entries.get_mut(key)?;
        thumbnail.used = clock;
        Some(thumbnail.texture.clone())
    }

    pub(in crate::app) fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub(in crate::app) fn insert(&mut self, key: String, texture: Option<egui::TextureHandle>) {
        self.clock += 1;
        let used = self.clock;
        let modified = loose_tag_modified(&key);
        self.entries.insert(
            key,
            Thumbnail {
                texture,
                used,
                modified,
            },
        );
        if self.entries.len() > THUMBNAIL_CACHE_CAP {
            self.evict_oldest();
        }
    }

    fn evict_oldest(&mut self) {
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(key, thumbnail)| (thumbnail.used, key.clone()))
            .collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        for (_, key) in by_age.into_iter().take(THUMBNAIL_EVICT_BATCH) {
            self.entries.remove(&key);
        }
    }

    /// Keep what is still right after the kit's entries changed: a thumbnail
    /// whose tag is still listed and whose file has not been modified since it
    /// was decoded. The libraries used to clear everything on any generation
    /// bump (a save elsewhere, a rename, the periodic refresh noticing one
    /// file) and re-read and re-decode every visible thumbnail.
    pub(in crate::app) fn revalidate(&mut self, still_listed: impl Fn(&str) -> bool) {
        self.entries.retain(|key, thumbnail| {
            still_listed(key) && loose_tag_modified(key) == thumbnail.modified
        });
    }
}

/// A bitmap decoded small enough to draw in a grid cell.
pub(in crate::app) struct ThumbnailImage {
    pub(in crate::app) rgba: Vec<u8>,
    pub(in crate::app) width: usize,
    pub(in crate::app) height: usize,
}

/// How many `cell`-wide thumbnails fit across `usable` points.
///
/// `n` cells occupy `n` widths and the `n - 1` gaps between them — there is no
/// gap after the last one — so the largest `n` that fits is
/// `(usable + gap) / (cell + gap)`, floored.
///
/// Counting a trailing gap that is never drawn, or leaving out the scrollbar the
/// caller subtracts before calling, both round this up by one and draw the
/// rightmost thumbnail half off the edge of the pane.
pub(in crate::app) fn grid_columns(usable: f32, cell: f32) -> usize {
    (((usable + CELL_GAP) / (cell + CELL_GAP)).floor() as usize).max(1)
}
/// How many decoded thumbnails are held at once.

/// Decode jobs allowed to run at once.

/// A bounded, least-recently-drawn thumbnail cache.

/// How many `cell`-wide thumbnails fit across `usable` points.

/// One kind of tag a thumbnail library lists, and how it draws them.
pub(in crate::app) trait ThumbnailSource: Sized + 'static {
    /// Salts the grid's scroll area, one per library.
    const ID_SALT: &'static str;
    /// The toolbar's count: "12 of 40 {PLURAL}".
    const PLURAL: &'static str;
    const SEARCH_HINT: &'static str;
    /// Shown while the kit is indexed and nothing is listed yet.
    const INDEXING: &'static str;
    /// Shown when the kit has none at all.
    const NONE_IN_KIT: &'static str;
    /// Shown when the search matches none: "No {SINGULAR} matches that search."
    const SINGULAR: &'static str;
    /// Prefix of the thumbnail textures' debug names.
    const TEXTURE_PREFIX: &'static str;
    /// The one item a cell's right-click menu offers. Choosing it sends
    /// [`CellAction::MenuAction`].
    const MENU_ITEM: &'static str;
    /// Which library this is, for the commands its cells send.
    const LIBRARY: Library;
    /// Why a worker sent no thumbnail, when it panicked.
    const CRASHED: &'static str;

    fn library(view: &KitView) -> &ThumbnailLibrary<Self>;
    fn library_mut(view: &mut KitView) -> &mut ThumbnailLibrary<Self>;
    /// Whether the library lists this tag.
    fn lists(entry: &TagEntry) -> bool;
    /// The group line under a cell's name.
    fn caption(entry: &TagEntry) -> &'static str;
    /// A cell's hover text, after its path.
    fn hover_hint(entry: &TagEntry) -> &'static str;
    /// Make the thumbnail, on a worker thread. The caller catches a panic.
    fn render(
        source: &TagSource,
        entry: &TagEntry,
        max_edge: u32,
    ) -> Result<ThumbnailImage, String>;
    fn message(
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
    ) -> WorkerMessage;
}

/// One kit's thumbnail library of one kind.
pub(in crate::app) struct ThumbnailLibrary<S> {
    pub(in crate::app) filter: String,
    pub(in crate::app) cell_size: f32,
    /// Every listed tag in the kit, snapshotted rather than re-scanned per
    /// frame.
    entries: Vec<TagEntry>,
    /// The kit generation `entries` was taken at, so a reload refreshes it.
    entries_for: Option<u64>,
    /// Indices into `entries` matching `filter`.
    matches: Vec<usize>,
    /// The query `matches` was computed for. Filtering tens of thousands of
    /// entries is cheap once and wasteful sixty times a second.
    matched_for: Option<String>,
    /// Shared with hover previews, which read it from inside draw code.
    pub(in crate::app) thumbnails: Arc<Mutex<ThumbnailCache>>,
    /// Keys with a job running, so a cell is not queued twice while its
    /// thread works.
    pub(in crate::app) pending: HashSet<String>,
    /// Set once the "scan the whole kit" request has gone out, so the library
    /// does not ask again every frame it is drawn.
    requested_scan: bool,
    source: PhantomData<S>,
}

impl<S> Default for ThumbnailLibrary<S> {
    fn default() -> Self {
        Self {
            filter: String::new(),
            cell_size: 0.0,
            entries: Vec::new(),
            entries_for: None,
            matches: Vec::new(),
            matched_for: None,
            thumbnails: Arc::default(),
            pending: HashSet::new(),
            requested_scan: false,
            source: PhantomData,
        }
    }
}

impl<S> ThumbnailLibrary<S> {
    fn cell_size(&self) -> f32 {
        if self.cell_size <= 0.0 {
            DEFAULT_CELL
        } else {
            self.cell_size.clamp(MIN_CELL, MAX_CELL)
        }
    }
}

/// What a grid cell asked for this frame. Both are parked rather than run on
/// the spot — see the fields they land in on [`ThumbnailLibrary`].
/// What a library cell can ask for.
pub(in crate::app) enum CellAction {
    /// Open the cell's tag (double-click).
    Open(String),
    /// The cell's right-click menu item.
    MenuAction(String),
}

/// The two thumbnail libraries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Library {
    Bitmaps,
    Models,
}

/// Draw one kit's library pane.
/// Its contents are refreshed beforehand, by
/// [`Baboon::refresh_thumbnail_library`]: that snapshots the kit's tags
/// and may start a scan.
pub(in crate::app) fn draw_thumbnail_library<S: ThumbnailSource>(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    library: &mut ThumbnailLibrary<S>,
) {
    let cell = library.cell_size();
    let total = library.matches.len();
    let all = library.entries.len();
    let scanning = cx.model.kits[kit_index].scanning_entries;

    draw_thumbnail_library_toolbar::<S>(ui, library, total, all, scanning);
    ui.separator();

    if all == 0 {
        ui.add_space(12.0);
        ui.label(
            RichText::new(if scanning {
                S::INDEXING
            } else {
                S::NONE_IN_KIT
            })
            .color(subtle_dark()),
        );
        return;
    }
    if total == 0 {
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!(
                "No {} matches that search. {all} in this workspace.",
                S::SINGULAR
            ))
            .color(subtle_dark()),
        );
        return;
    }

    draw_thumbnail_grid::<S>(cx, ui, kit_index, library, cell, total);
}

fn draw_thumbnail_library_toolbar<S: ThumbnailSource>(
    ui: &mut Ui,
    library: &mut ThumbnailLibrary<S>,
    shown: usize,
    total: usize,
    scanning: bool,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Search").color(subtle_dark()));
        ui.add(
            egui::TextEdit::singleline(&mut library.filter)
                .hint_text(placeholder_text(S::SEARCH_HINT))
                .desired_width(240.0),
        )
        .on_hover_text(
            "Space is AND, | is OR, ^foo and foo$ anchor to the start and end of the name — \
             the same search the tag browser uses.",
        );
        if ui.button("Clear").clicked() {
            library.filter.clear();
        }

        ui.separator();
        ui.label(RichText::new("Size").color(subtle_dark()));
        let mut cell = library.cell_size();
        if ui
            .add(
                egui::Slider::new(&mut cell, MIN_CELL..=MAX_CELL)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            )
            .changed()
        {
            library.cell_size = cell;
        }
        if ui.button("Reset").clicked() {
            library.cell_size = DEFAULT_CELL;
        }

        ui.separator();
        let count = if shown == total {
            format!("{total} {}", S::PLURAL)
        } else {
            format!("{shown} of {total} {}", S::PLURAL)
        };
        ui.label(RichText::new(count).color(subtle_dark()));
        if scanning {
            ui.spinner();
            ui.label(RichText::new("indexing…").color(subtle_dark()));
        }
    });
}

fn draw_thumbnail_grid<S: ThumbnailSource>(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    library: &mut ThumbnailLibrary<S>,
    cell: f32,
    total: usize,
) {
    let row_height = cell + CELL_CAPTION + CELL_GAP;
    // Reserve the scrollbar before dividing. `available_width` here is the
    // width *outside* the scroll area, and the bar is taken from the inside
    // — count the full width and the rightmost column is drawn half off the
    // edge, which is what a wide window made obvious.
    let usable = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(cell);
    let columns = grid_columns(usable, cell);
    let rows = total.div_ceil(columns);

    // Row virtualisation is what makes this affordable: `show_rows` hands
    // back only the visible band, so a kit with twenty thousand bitmaps
    // lays out the thirty on screen and queues jobs for those alone.
    let mut action: Option<CellAction> = None;
    let mut wanted: Vec<String> = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt((S::ID_SALT, kit_index))
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows, |ui, row_range| {
            // The gap becomes the only spacing in play, horizontally and
            // vertically. egui's default `item_spacing` would otherwise be
            // added between every cell on top of it — the column arithmetic
            // above would be short by one gap per cell, and each row would
            // stand taller than the `row_height` `show_rows` is scrolling
            // by, so the grid would drift out of step with its scrollbar.
            ui.spacing_mut().item_spacing = Vec2::new(CELL_GAP, 0.0);
            for row in row_range {
                ui.horizontal(|ui| {
                    for column in 0..columns {
                        let Some(index) = row
                            .checked_mul(columns)
                            .and_then(|start| start.checked_add(column))
                            .filter(|index| *index < total)
                        else {
                            break;
                        };
                        if let Some(requested) = draw_thumbnail_cell::<S>(
                            ui,
                            library,
                            index,
                            cell,
                            &mut wanted,
                        ) {
                            action = Some(requested);
                        }
                    }
                });
                ui.add_space(CELL_GAP);
            }
        });

    // Requested at twice the cell's point size, so the thumbnail still looks
    // right after the slider grows a little and on a high-DPI display.
    let max_edge = ((cell * 2.0).round() as u32).max(MIN_CELL as u32);
    let entries = wanted
        .into_iter()
        .filter_map(|key| {
            library
                .entries
                .iter()
                .find(|entry| entry.key == key)
                .cloned()
        })
        .collect();
    queue_thumbnails::<S>(cx, kit_index, library, entries, max_edge);
    // Sent rather than opened here. This runs inside `tree.ui`, and
    // `draw_tag_tiles` has moved the kit's `tag_tree` out for the duration —
    // so opening a tab here would insert it into the placeholder that is
    // thrown away when the real tree is put back. A command runs once the
    // frame's drawing is over, with the tree back in place.
    if let Some(action) = action {
        cx.send(BrowserCommand::LibraryCell {
            kit: cx.model.kits[kit_index].id,
            library: S::LIBRARY,
            action,
        });
    }
}

/// Which of `entries` a folder pane's Asset Browser shows, in order, as
/// indices. Worked out again only when the entries, the folder, the sort or
/// the asset kinds change: done every frame, it re-split the folder path for
/// every entry and lowercased each name to sort them, about 19 ms a frame at
/// the root of the Halo 3 kit. `entries_signature` names the entries; without
/// one nothing is kept.
fn asset_grid_order(
    ui: &Ui,
    pane_key: &str,
    pane: &FolderBrowserState,
    entries: &[TagEntry],
    entries_signature: Option<u64>,
) -> std::sync::Arc<Vec<usize>> {
    use std::hash::{Hash, Hasher};
    let key = entries_signature.map(|signature| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            signature,
            entries.len(),
            &pane.rel_path,
            pane.sort,
            pane.asset_bitmaps,
            pane.asset_models,
        )
            .hash(&mut hasher);
        hasher.finish()
    });
    let id = egui::Id::new(("asset_grid_order", pane_key));
    if let Some(key) = key
        && let Some((cached, order)) = ui.data(|data| data.get_temp::<(u64, std::sync::Arc<Vec<usize>>)>(id))
        && cached == key
    {
        return order;
    }
    let mut order: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            crate::core::source::entry_is_beneath_folder(entry, &pane.rel_path)
                && ((pane.asset_bitmaps && Bitmaps::lists(entry))
                    || (pane.asset_models && Models::lists(entry)))
        })
        .map(|(index, _)| index)
        .collect();
    match pane.sort {
        BrowserSort::Natural => {}
        BrowserSort::Name => order.sort_by_cached_key(|&index| {
            tag_leaf_name(&entries[index].display_path).to_ascii_lowercase()
        }),
        BrowserSort::Type => order.sort_by_cached_key(|&index| {
            (
                format_group_tag(entries[index].group_tag),
                tag_leaf_name(&entries[index].display_path).to_ascii_lowercase(),
            )
        }),
    }
    let order = std::sync::Arc::new(order);
    if let Some(key) = key {
        ui.data_mut(|data| data.insert_temp(id, (key, order.clone())));
    }
    order
}

/// A mixed folder grid using the same cells, caches, workers and commands as
/// the standalone bitmap and model libraries: a folder pane's Asset Browser.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_folder_asset_grid(
    cx: &Ctx,
    ui: &mut Ui,
    kit_index: usize,
    pane_key: &str,
    pane: &FolderBrowserState,
    entries: &[TagEntry],
    entries_signature: Option<u64>,
    bitmaps: &mut ThumbnailLibrary<Bitmaps>,
    models: &mut ThumbnailLibrary<Models>,
) {
    let order = asset_grid_order(ui, pane_key, pane, entries, entries_signature);
    let visible: Vec<&TagEntry> = order.iter().map(|&index| &entries[index]).collect();
    let game = cx.model.kits[kit_index]
        .source
        .as_ref()
        .and_then(|source| source.game);
    if visible.is_empty() {
        ui.label(RichText::new("No matching assets in this folder").color(subtle_dark()));
        return;
    }
    let mut wanted_bitmaps = Vec::new();
    let mut wanted_models = Vec::new();
    let mut action = None;
    let cell = pane.asset_cell_size.clamp(MIN_CELL, MAX_CELL);
    let columns = grid_columns(
        (ui.available_width() - ui.spacing().scroll.allocated_width()).max(cell),
        cell,
    );
    let height = cell + CELL_CAPTION + CELL_GAP;
    ScrollArea::vertical()
        .id_salt(("folder_assets", pane_key))
        .auto_shrink([false, false])
        .show_rows(ui, height, visible.len().div_ceil(columns), |ui, rows| {
            ui.spacing_mut().item_spacing = Vec2::new(CELL_GAP, 0.0);
            for row in rows {
                ui.horizontal(|ui| {
                    for entry in visible.iter().skip(row * columns).take(columns) {
                        let cell_action = if Bitmaps::lists(entry) {
                            let wanted = &mut wanted_bitmaps;
                            draw_thumbnail_entry(ui, bitmaps, entry, cell, wanted, Some(game))
                                .map(|action| (Bitmaps::LIBRARY, action))
                        } else {
                            let wanted = &mut wanted_models;
                            draw_thumbnail_entry(ui, models, entry, cell, wanted, Some(game))
                                .map(|action| (Models::LIBRARY, action))
                        };
                        if cell_action.is_some() {
                            action = cell_action;
                        }
                    }
                });
                ui.add_space(CELL_GAP);
            }
        });
    let wanted = |keys: Vec<String>| -> Vec<TagEntry> {
        keys.into_iter()
            .filter_map(|key| visible.iter().find(|entry| entry.key == key))
            .map(|entry| (*entry).clone())
            .collect()
    };
    let (wanted_bitmaps, wanted_models) = (wanted(wanted_bitmaps), wanted(wanted_models));
    // Thumbnails twice the cell's edge, so they stay sharp on high-DPI screens.
    let edge = ((cell * 2.0).round() as u32).max(MIN_CELL as u32);
    queue_thumbnails::<Bitmaps>(cx, kit_index, bitmaps, wanted_bitmaps, edge);
    queue_thumbnails::<Models>(cx, kit_index, models, wanted_models, edge);
    if let Some((library, action)) = action {
        cx.send(BrowserCommand::LibraryCell {
            kit: cx.model.kits[kit_index].id,
            library,
            action,
        });
    }
}

/// One grid cell, and whatever the user asked it for.
fn draw_thumbnail_cell<S: ThumbnailSource>(
    ui: &mut Ui,
    library: &mut ThumbnailLibrary<S>,
    index: usize,
    cell: f32,
    wanted: &mut Vec<String>,
) -> Option<CellAction> {
    let entry_index = *library.matches.get(index)?;
    let entry = library.entries.get(entry_index)?.clone();
    draw_thumbnail_entry(ui, library, &entry, cell, wanted, None)
}

/// One cell for `entry`, from `library`'s caches. `type_badge` marks it with
/// its group's icon in the kit's game, for a grid that mixes bitmaps and
/// models.
fn draw_thumbnail_entry<S: ThumbnailSource>(
    ui: &mut Ui,
    library: &mut ThumbnailLibrary<S>,
    entry: &TagEntry,
    cell: f32,
    wanted: &mut Vec<String>,
    type_badge: Option<Option<GameId>>,
) -> Option<CellAction> {
    let (key, display_path) = (entry.key.clone(), entry.display_path.clone());

    let cached = library
        .thumbnails
        .lock()
        .ok()
        .and_then(|mut thumbnails| thumbnails.get(&key));
    // Cached as `None` is a thumbnail that could not be made: it is done,
    // not loading, so it must not spin (and repaint) for as long as it is
    // on screen.
    let failed = matches!(cached, Some(None));
    let texture = match cached {
        Some(texture) => texture,
        None => {
            // Not made yet. Ask for it, draw the placeholder, and let the
            // worker's reply repaint the frame.
            if !library.pending.contains(&key) {
                wanted.push(key.clone());
            }
            None
        }
    };

    let size = Vec2::new(cell, cell + CELL_CAPTION);
    // `click_and_drag`, so a cell is both a target to open and a source to
    // drag. The payload is the browser row's own `DraggedTagRef` — the
    // shader bitmap rows and Foundation reference cells already accept it,
    // and a shader slot already checks for the `bitm` group — so dragging a
    // thumbnail onto a reference needs nothing on the drop side.
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    response.dnd_set_drag_payload(DraggedTagRef {
        group_tag: entry.group_tag,
        input: entry_reference_input(entry),
        rel_path: entry_rel_path(entry),
        file_path: entry_loose_file(entry),
    });
    let (caption, hover_hint) = (S::caption(entry), S::hover_hint(entry));
    let image_rect = egui::Rect::from_min_size(rect.min, Vec2::splat(cell));
    ui.painter()
        .rect_filled(image_rect, 0.0, foundation_input());
    if response.hovered() {
        ui.painter()
            .rect_stroke(
                image_rect,
                0.0,
                Stroke::new(1.0_f32, foundation_blue()),
                egui::StrokeKind::Middle,
            );
    } else {
        ui.painter()
            .rect_stroke(
                image_rect,
                0.0,
                Stroke::new(1.0_f32, foundation_input_edge()),
                egui::StrokeKind::Middle,
            );
    }

    match texture {
        Some(texture) => {
            let drawn = fit_within(texture.size_vec2(), cell - 2.0);
            let at = egui::Rect::from_center_size(image_rect.center(), drawn);
            egui::Image::new(&texture).paint_at(ui, at);
        }
        None if failed => {
            ui.painter().text(
                image_rect.center(),
                Align2::CENTER_CENTER,
                "No preview",
                FontId::proportional(11.0),
                subtle_dark(),
            );
        }
        None => {
            crate::app::shell::loading::paint_loading_rings(ui, image_rect);
        }
    }

    if let Some(game) = type_badge {
        let badge = egui::Rect::from_min_size(
            image_rect.right_bottom() - Vec2::splat(22.0),
            Vec2::splat(20.0),
        );
        ui.painter().rect_filled(badge, 2.0, foundation_input());
        paint_tag_icon_at(ui, Some(entry.group_tag), game, badge.shrink(2.0));
    }
    let name = tag_leaf_name(&display_path);
    ui.painter().text(
        egui::Pos2::new(rect.center().x, image_rect.bottom() + 8.0),
        Align2::CENTER_CENTER,
        truncate_for_cell(&name, cell),
        FontId::proportional(11.5),
        text_dark(),
    );
    ui.painter().text(
        egui::Pos2::new(rect.center().x, image_rect.bottom() + 21.0),
        Align2::CENTER_CENTER,
        caption,
        FontId::proportional(10.0),
        subtle_dark(),
    );
    // No trailing space: the row's `item_spacing` puts the gap *between*
    // cells and none after the last one, which is what the column count
    // assumes. Adding it here too would overflow the row by one gap per
    // cell and push the rightmost column off the edge.

    // The name follows the cursor while dragging, as the browser rows do —
    // the thumbnail is left behind, so without this there is nothing to say
    // which tag is in flight.
    if response.dragged()
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        egui::Area::new(ui.make_persistent_id((S::ID_SALT, "drag_preview", &key)))
            .order(egui::Order::Tooltip)
            // Never in the hit-test: a fast drag can put the pointer
            // inside the stale preview, which would block the drop target.
            .interactable(false)
            .fixed_pos(pointer + Vec2::new(12.0, 12.0))
            .show(ui.ctx(), |ui| {
                ui.label(RichText::new(&name).color(text_dark()));
            });
    }

    // Double-click, not click: a single click on a grid this dense is far
    // too easy to do by accident, and every open parses a tag and adds a tab.
    let mut action = response
        .double_clicked()
        .then(|| CellAction::Open(key.clone()));

    // The browser tree's own menu styling, so a right-click here looks like
    // a right-click anywhere else in Baboon.
    context_menu(&response, |ui| {
        style_tag_context_menu(ui);
        if context_menu_button(ui, S::MENU_ITEM).clicked() {
            action = Some(CellAction::MenuAction(key.clone()));
            close_menu(ui);
        }
    });

    // Suppressed while that menu is open: a tooltip would otherwise sit over
    // the item the cursor is on. Not `on_hover_text`: an egui tooltip would
    // block the drag this cell offers (`hover_tooltip_beside_pointer`).
    if !response.context_menu_opened() {
        hover_tooltip_beside_pointer(ui, &response, &format!("{display_path}\n\n{hover_hint}"));
    }
    action
}

impl Baboon {
    /// Snapshot the kit's listed tags and recompute the filter, both only when
    /// something they depend on has actually changed.
    pub(in crate::app) fn refresh_thumbnail_library<S: ThumbnailSource>(
        &mut self,
        kit_index: usize,
        ctx: &egui::Context,
    ) {
        let generation = self.model.kits[kit_index].generation;
        let stale = S::library(&self.views[self.model.kits[kit_index].id]).entries_for != Some(generation);
        if stale {
            let entries: Vec<TagEntry> = self.model.kits[kit_index]
                .source
                .as_ref()
                .map(|source| source.full_entry_set())
                .unwrap_or_default()
                .iter()
                .filter(|entry| S::lists(entry))
                .cloned()
                .collect();
            let library = S::library_mut(&mut self.views[self.model.kits[kit_index].id]);
            let listed: HashSet<&str> = entries.iter().map(|entry| entry.key.as_str()).collect();
            if let Ok(mut thumbnails) = library.thumbnails.lock() {
                thumbnails.revalidate(|key| listed.contains(key));
            }
            drop(listed);
            library.entries = entries;
            library.entries_for = Some(generation);
            library.matched_for = None;
            // A new source gets to ask for its own scan; the flag only exists
            // to stop the request repeating every frame within one source.
            library.requested_scan = false;
        }

        // A loose kit only holds the folders the browser has expanded until the
        // full scan runs, so without this the library would show a fraction of
        // the kit and give no clue why. Asked for once, the same way the
        // browser asks when Groups view or a search needs it.
        let needs_scan = self.model.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| source.all_entries.is_empty())
            && !self.model.kits[kit_index].scanning_entries
            && !S::library(&self.views[self.model.kits[kit_index].id]).requested_scan;
        if needs_scan {
            S::library_mut(&mut self.views[self.model.kits[kit_index].id]).requested_scan = true;
            self.begin_scan_all_entries_in(kit_index, ctx.clone(), "Indexing tags...");
        }

        let library = S::library_mut(&mut self.views[self.model.kits[kit_index].id]);
        if library.matched_for.as_deref() != Some(library.filter.as_str()) {
            let filter = library.filter.trim().to_owned();
            library.matches = library
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| filter.is_empty() || entry_matches(entry, &filter))
                .map(|(index, _)| index)
                .collect();
            library.matched_for = Some(library.filter.clone());
        }
    }

    pub(in crate::app) fn handle_thumbnail_ready<S: ThumbnailSource>(
        &mut self,
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            // The kit closed while this ran.
            return true;
        };
        // Clear the in-flight marker before deciding whether the result is
        // stale. Returning first, as this did, left it set after any generation
        // bump that landed mid-job, so the work was never asked for again.
        S::library_mut(&mut self.views[self.model.kits[kit_index].id])
            .pending
            .remove(&key);
        if self.model.resolve_stamp(stamp).is_none() {
            // Reloaded while this ran: the thumbnail is of the old source.
            return true;
        }
        // A failure is cached as `None` rather than dropped: an empty or
        // unsupported tag would otherwise be retried every frame it stays on
        // screen, which is the one way this grid could still stall.
        let texture = match result {
            Ok(image) => Some(ctx.load_texture(
                format!("{}:{key}", S::TEXTURE_PREFIX),
                egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba),
                egui::TextureOptions::LINEAR,
            )),
            Err(_) => None,
        };
        let library = S::library_mut(&mut self.views[self.model.kits[kit_index].id]);
        if let Ok(mut thumbnails) = library.thumbnails.lock() {
            thumbnails.insert(key, texture);
        }
        false
    }
}

/// Start thumbnail jobs for `entries` that have none, up to the in-flight
/// bound.
pub(in crate::app) fn queue_thumbnails<S: ThumbnailSource>(
    cx: &Ctx,
    kit_index: usize,
    library: &mut ThumbnailLibrary<S>,
    entries: Vec<TagEntry>,
    max_edge: u32,
) {
    if entries.is_empty() {
        return;
    }
    let Some(source) = cx.model.kits[kit_index]
        .source
        .as_ref()
        .map(|source| source.source.clone())
    else {
        return;
    };
    let stamp = KitStamp {
        kit: cx.model.kits[kit_index].id,
        generation: cx.model.kits[kit_index].generation,
    };

    for entry in entries {
        let key = entry.key.clone();
        let cached = library
            .thumbnails
            .lock()
            .is_ok_and(|thumbnails| thumbnails.contains(&key));
        if cached {
            continue;
        }
        if library.pending.len() >= MAX_DECODES_IN_FLIGHT {
            break;
        }
        if !library.pending.insert(key.clone()) {
            continue;
        }

        // Through `spawn_worker` because tags have panicked the bitmap
        // decoders and the geometry parser before: a thread that panics
        // never sends, so `pending` would keep its key and one of the four
        // slots would be gone for good. Four such tags stopped the library
        // and every hover preview.
        let source = source.clone();
        let panic_key = key.clone();
        cx.spawn(
            move || S::message(stamp, key, S::render(&source, &entry, max_edge)),
            move |_| S::message(stamp, panic_key, Err(S::CRASHED.to_owned())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One grid, two libraries: each lists only its own kind of tag, searches
    // within it, and queues thumbnail jobs only for what it shows.

    fn entry(display_path: &str, group: &[u8; 4]) -> TagEntry {
        TagEntry {
            key: format!("file:{display_path}"),
            display_path: display_path.to_owned(),
            group_tag: u32::from_be_bytes(*group),
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    /// A kit holding bitmaps, render models, a gbxmodel, and tags of neither kind.
    fn app_with_mixed_kit() -> Baboon {
        let mut entries = vec![
            entry("objects/warthog/warthog.render_model", b"mode"),
            entry("objects/ghost/ghost.gbxmodel", b"mod2"),
            entry("objects/warthog/warthog.model", b"hlmt"),
            entry("objects/warthog/warthog.vehicle", b"vehi"),
        ];
        for index in 0..40 {
            entries.push(entry(&format!("textures/grass_{index:02}.bitmap"), b"bitm"));
        }
        let mut app = Baboon::for_test();
        app.install_loaded_source(LoadedSourceData {
            label: "mixed kit".to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("<no kit root>"),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: entries.clone(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: entries,
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: true,
            chosen_kit_layout: None,
        });
        app
    }

    fn listed<S: ThumbnailSource>(app: &Baboon) -> Vec<String> {
        let library = S::library(&app.views[app.model.kits[0].id]);
        library
            .matches
            .iter()
            .map(|&index| library.entries[index].display_path.clone())
            .collect()
    }

    #[test]
    fn each_library_lists_only_its_own_kind_of_tag() {
        let mut app = app_with_mixed_kit();
        let ctx = egui::Context::default();
        app.refresh_thumbnail_library::<Models>(0, &ctx);
        app.refresh_thumbnail_library::<Bitmaps>(0, &ctx);

        assert_eq!(
            listed::<Models>(&app),
            [
                "objects/warthog/warthog.render_model",
                "objects/ghost/ghost.gbxmodel"
            ],
            "render geometry only: not the .model wrapper, not the vehicle"
        );
        let bitmaps = listed::<Bitmaps>(&app);
        assert_eq!(bitmaps.len(), 40);
        assert!(bitmaps.iter().all(|path| path.ends_with(".bitmap")));
    }

    #[test]
    fn a_search_narrows_only_its_own_library() {
        let mut app = app_with_mixed_kit();
        let ctx = egui::Context::default();
        app.views[app.model.kits[0].id].model_browser.filter = "ghost".to_owned();
        app.refresh_thumbnail_library::<Models>(0, &ctx);
        app.refresh_thumbnail_library::<Bitmaps>(0, &ctx);

        assert_eq!(listed::<Models>(&app), ["objects/ghost/ghost.gbxmodel"]);
        assert_eq!(
            listed::<Bitmaps>(&app).len(),
            40,
            "the other library's search is its own"
        );
    }

    /// Drawing a library queues thumbnail jobs for the cells it shows, bounded,
    /// and only for its own tags.
    fn draws_and_queues_its_own<S: ThumbnailSource>(expected_suffix: &str) {
        let mut app = app_with_mixed_kit();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            ..Default::default()
        };
        let _ = crate::app::run_ui_test(&ctx, input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                app.refresh_thumbnail_library::<S>(0, &ctx);
                let kit = app.model.kits[0].id;
                draw_thumbnail_library::<S>(
                    &cx!(app, &ctx),
                    ui,
                    0,
                    S::library_mut(&mut app.views[kit]),
                );
            });
        });
        let pending = &S::library(&app.views[app.model.kits[0].id]).pending;
        assert!(!pending.is_empty(), "no thumbnail was asked for");
        assert!(pending.len() <= MAX_DECODES_IN_FLIGHT);
        assert!(
            pending.iter().all(|key| key.ends_with(expected_suffix)),
            "queued another library's tag: {pending:?}"
        );
    }

    #[test]
    fn the_bitmap_library_queues_bitmaps() {
        draws_and_queues_its_own::<Bitmaps>(".bitmap");
    }

    #[test]
    fn the_model_library_queues_models() {
        draws_and_queues_its_own::<Models>("model");
    }

    /// How long egui may wait before the next frame, once the bitmap library has
    /// settled with every bitmap's thumbnail either pending or failed.
    fn repaint_delay_with_thumbnails(failed: bool) -> std::time::Duration {
        let mut app = app_with_mixed_kit();
        if failed {
            let mut thumbnails = Bitmaps::library(&app.views[app.model.kits[0].id]).thumbnails.lock().unwrap();
            for index in 0..40 {
                thumbnails.insert(format!("file:textures/grass_{index:02}.bitmap"), None);
            }
        }
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            ..Default::default()
        };
        // egui repaints its first frames by itself, and the scrollbar fades in
        // over time that only passes if the input says so. Once both settle, the
        // frame says what the library itself asked for.
        let mut delay = std::time::Duration::ZERO;
        for frame in 0..3 {
            let input = egui::RawInput {
                time: Some(frame as f64),
                ..input.clone()
            };
            let output = crate::app::run_ui_test(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    app.refresh_thumbnail_library::<Bitmaps>(0, &ctx);
                    let kit = app.model.kits[0].id;
                    draw_thumbnail_library::<Bitmaps>(
                        &cx!(app, &ctx),
                        ui,
                        0,
                        Bitmaps::library_mut(&mut app.views[kit]),
                    );
                });
            });
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        delay
    }

    /// A failed thumbnail is finished, not loading: a screen of them must not
    /// spin, or keep the whole window repainting for as long as they are shown.
    #[test]
    fn failed_thumbnails_do_not_keep_the_window_repainting() {
        let pending = repaint_delay_with_thumbnails(false);
        assert!(
            pending <= std::time::Duration::from_millis(16),
            "pending thumbnails should animate: {pending:?}"
        );
        let failed = repaint_delay_with_thumbnails(true);
        assert!(
            failed > std::time::Duration::from_secs(1),
            "failed thumbnails still repaint every {failed:?}"
        );
    }

    fn folder_pane() -> FolderBrowserState {
        FolderBrowserState {
            rel_path: "objects/brute".into(),
            label: "brute".into(),
            filter: String::new(),
            focus_search: false,
            mode: BrowserMode::Folders,
            sort: BrowserSort::Name,
            cached_generation: 0,
            cached_source_len: 0,
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            group_tree_for: None,
            filter_cache: FilterCache::default(),
            date_cache: FolderDateCache::default(),
            table_layout: FolderTableLayout::default(),
            search_scope: BrowserSearchScope::default(),
            assets_view: true,
            asset_bitmaps: true,
            asset_models: true,
            asset_cell_size: DEFAULT_CELL,
        }
    }

    fn loose_entry(path: &str, group: &[u8; 4]) -> TagEntry {
        TagEntry {
            key: path.into(),
            display_path: path.into(),
            group_tag: u32::from_be_bytes(*group),
            group_name: None,
            location: TagEntryLocation::LooseFile(path.into()),
        }
    }

    /// The Asset Browser's order is worked out once per set of entries,
    /// folder, sort and asset kinds, and again when any of them changes.
    #[test]
    fn the_asset_order_is_kept_until_its_inputs_change() {
        let entries = vec![
            loose_entry("objects/brute/zeta.bitmap", b"bitm"),
            loose_entry("objects/brute/alpha.bitmap", b"bitm"),
            loose_entry("objects/other/beta.bitmap", b"bitm"),
        ];
        let ctx = egui::Context::default();
        let mut pane = folder_pane();
        let mut orders = Vec::new();
        let mut order = |pane: &FolderBrowserState, signature| {
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                orders.push(asset_grid_order(ui, "pane", pane, &entries, signature));
            });
            orders.last().unwrap().clone()
        };
        let first = order(&pane, Some(1));
        assert_eq!(*first, [1, 0], "beneath the folder, by name");
        assert!(std::sync::Arc::ptr_eq(&first, &order(&pane, Some(1))), "kept");
        pane.sort = BrowserSort::Natural;
        assert_eq!(*order(&pane, Some(1)), [0, 1], "a new sort orders again");
        assert!(!std::sync::Arc::ptr_eq(&order(&pane, None), &order(&pane, None)), "nothing kept without a signature");
    }

    /// The Asset Browser grid shows only what is beneath its folder, of the
    /// types ticked, and queues a thumbnail for each through its own library.
    #[test]
    fn mixed_grid_is_folder_scoped_and_type_filters_work() {
        let entries = vec![
            loose_entry("objects/brute/grass.bitmap", b"bitm"),
            loose_entry("objects/brute/nested/brute.render_model", b"mode"),
            loose_entry("objects/brute/brute.biped", b"bipd"),
            loose_entry("objects/brute_other/other.bitmap", b"bitm"),
        ];
        for (bitmap, model, expected_bitmaps, expected_models) in [
            (true, true, 1, 1),
            (true, false, 1, 0),
            (false, true, 0, 1),
            (false, false, 0, 0),
        ] {
            let mut app = Baboon::for_test();
            app.install_loaded_source(LoadedSourceData {
                label: "brute".to_owned(),
                source: TagSource::LooseFolder {
                    root: PathBuf::from("<no kit root>"),
                    game: None,
                    definitions_root: PathBuf::new(),
                },
                names: TagNameIndex::default(),
                game: None,
                entries: entries.clone(),
                tree: TagTree::default(),
                group_tree: TagTree::default(),
                all_entries: entries.clone(),
                reverse_dependencies: None,
                initial_tag: None,
                key_hints: Default::default(),
                complete_scan: true,
                chosen_kit_layout: None,
            });
            let ctx = egui::Context::default();
            let mut pane = folder_pane();
            pane.asset_bitmaps = bitmap;
            pane.asset_models = model;
            let kit = app.model.kits[0].id;
            let _ = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(900.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let view = &mut app.views[kit];
                        draw_folder_asset_grid(
                            &cx!(app, &ctx),
                            ui,
                            0,
                            "test",
                            &pane,
                            &entries,
                            None,
                            &mut view.bitmap_browser,
                            &mut view.model_browser,
                        );
                    });
                },
            );
            let view = &app.views[kit];
            let queued: Vec<&String> =
                view.bitmap_browser.pending.iter().chain(&view.model_browser.pending).collect();
            assert_eq!(view.bitmap_browser.pending.len(), expected_bitmaps);
            assert_eq!(view.model_browser.pending.len(), expected_models);
            assert!(
                queued.iter().all(|key| key.contains("objects/brute/")),
                "queued outside the folder: {queued:?}"
            );
        }
    }

    #[test]
    fn folder_grid_search_uses_the_same_scoped_cache_as_the_tree() {
        let mut pane = folder_pane();
        let entries = vec![
            loose_entry("objects/brute/grass.bitmap", b"bitm"),
            loose_entry("objects/brute/brute.render_model", b"mode"),
            loose_entry("objects/elite/grass.bitmap", b"bitm"),
        ];
        let keywords =
            std::collections::BTreeMap::from([(entries[0].key.clone(), vec!["wip".into()])]);
        pane.search_scope = BrowserSearchScope {
            tags: false,
            folders: false,
            keywords: true,
        };
        pane.filter_cache.refresh_scoped(
            1,
            "wip",
            &entries,
            false,
            &pane.rel_path,
            None,
            pane.search_scope,
            &keywords,
        );
        assert_eq!(pane.filter_cache.entries.len(), 1);
        assert_eq!(pane.filter_cache.entries[0].key, entries[0].key);
    }
}
