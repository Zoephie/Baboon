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
    let path = key.strip_prefix("file:")?;
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
    /// The one item a cell's right-click menu offers. Choosing it parks the
    /// key in [`ThumbnailLibrary::pending_menu_action`].
    const MENU_ITEM: &'static str;
    /// Why a worker sent no thumbnail, when it panicked.
    const CRASHED: &'static str;

    fn library(kit: &Kit) -> &ThumbnailLibrary<Self>;
    fn library_mut(kit: &mut Kit) -> &mut ThumbnailLibrary<Self>;
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
    /// A double-clicked cell waiting to be opened as a tab.
    ///
    /// The grid draws inside `tree.ui`, where the kit's `tag_tree` has been
    /// moved out; opening there writes the tab into a placeholder that is
    /// discarded. `draw_tag_tiles` takes this once the tree is back.
    pub(in crate::app) pending_open: Option<String>,
    /// A cell whose right-click menu item ([`ThumbnailSource::MENU_ITEM`])
    /// was chosen. Parked for the same reason, and for the Bitmap Library one
    /// more: its extract opens a native folder picker, which blocks the thread
    /// until the user answers it. Doing that mid-walk would stall the frame
    /// with the kit's tag tree still moved out of it.
    pub(in crate::app) pending_menu_action: Option<String>,
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
            pending_open: None,
            pending_menu_action: None,
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
enum CellAction {
    Open(String),
    MenuAction(String),
}

impl Baboon {
    /// Draw one kit's library pane.
    pub(super) fn draw_thumbnail_library<S: ThumbnailSource>(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
    ) {
        self.refresh_thumbnail_library::<S>(kit_index, ctx);

        let library = S::library(&self.kits[kit_index]);
        let cell = library.cell_size();
        let total = library.matches.len();
        let all = library.entries.len();
        let scanning = self.kits[kit_index].scanning_entries;

        self.draw_thumbnail_library_toolbar::<S>(ui, kit_index, total, all, scanning);
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

        self.draw_thumbnail_grid::<S>(ui, ctx, kit_index, cell, total);
    }

    fn draw_thumbnail_library_toolbar<S: ThumbnailSource>(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        shown: usize,
        total: usize,
        scanning: bool,
    ) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Search").color(subtle_dark()));
            let library = S::library_mut(&mut self.kits[kit_index]);
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
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
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
                            if let Some(requested) = self.draw_thumbnail_cell::<S>(
                                ui,
                                kit_index,
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
        let library = S::library(&self.kits[kit_index]);
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
        self.queue_thumbnails::<S>(kit_index, entries, max_edge, ctx);
        // Parked rather than opened here. This runs inside `tree.ui`, and
        // `draw_tag_tiles` has moved the kit's `tag_tree` out for the duration —
        // so `open_tag_pane` would insert the new tab into the placeholder that
        // is thrown away when the real tree is put back. The tag loaded and no
        // tab ever appeared. `draw_tag_tiles` drains these after the walk,
        // which is where every other pane mutation is applied for the same
        // reason.
        let library = S::library_mut(&mut self.kits[kit_index]);
        match action {
            Some(CellAction::Open(key)) => library.pending_open = Some(key),
            Some(CellAction::MenuAction(key)) => library.pending_menu_action = Some(key),
            None => {}
        }
    }

    /// One grid cell, and whatever the user asked it for.
    fn draw_thumbnail_cell<S: ThumbnailSource>(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        index: usize,
        cell: f32,
        wanted: &mut Vec<String>,
    ) -> Option<CellAction> {
        let library = S::library_mut(&mut self.kits[kit_index]);
        let entry_index = *library.matches.get(index)?;
        let entry = library.entries.get(entry_index)?;
        let (key, display_path) = (entry.key.clone(), entry.display_path.clone());

        let cached = library
            .thumbnails
            .lock()
            .ok()
            .and_then(|mut thumbnails| thumbnails.get(&key));
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
                .rect_stroke(image_rect, 0.0, Stroke::new(1.0, foundation_blue()));
        } else {
            ui.painter()
                .rect_stroke(image_rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
        }

        match texture {
            Some(texture) => {
                let drawn = fit_within(texture.size_vec2(), cell - 2.0);
                let at = egui::Rect::from_center_size(image_rect.center(), drawn);
                egui::Image::new(&texture).paint_at(ui, at);
            }
            None => {
                crate::app::ui::paint_loading_rings(ui, image_rect);
            }
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
        response.context_menu(|ui| {
            style_tag_context_menu(ui);
            if context_menu_button(ui, S::MENU_ITEM).clicked() {
                action = Some(CellAction::MenuAction(key.clone()));
                ui.close_menu();
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

    /// Snapshot the kit's listed tags and recompute the filter, both only when
    /// something they depend on has actually changed.
    pub(super) fn refresh_thumbnail_library<S: ThumbnailSource>(
        &mut self,
        kit_index: usize,
        ctx: &egui::Context,
    ) {
        let generation = self.kits[kit_index].generation;
        let stale = S::library(&self.kits[kit_index]).entries_for != Some(generation);
        if stale {
            let entries: Vec<TagEntry> = self.kits[kit_index]
                .source
                .as_ref()
                .map(|source| source.full_entry_set())
                .unwrap_or_default()
                .iter()
                .filter(|entry| S::lists(entry))
                .cloned()
                .collect();
            let library = S::library_mut(&mut self.kits[kit_index]);
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
        let needs_scan = self.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| source.all_entries.is_empty())
            && !self.kits[kit_index].scanning_entries
            && !S::library(&self.kits[kit_index]).requested_scan;
        if needs_scan {
            S::library_mut(&mut self.kits[kit_index]).requested_scan = true;
            self.begin_scan_all_entries_in(kit_index, ctx.clone(), "Indexing tags...");
        }

        let library = S::library_mut(&mut self.kits[kit_index]);
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

    /// Start thumbnail jobs for `entries` that have none, up to the in-flight
    /// bound.
    pub(super) fn queue_thumbnails<S: ThumbnailSource>(
        &mut self,
        kit_index: usize,
        entries: Vec<TagEntry>,
        max_edge: u32,
        ctx: &egui::Context,
    ) {
        if entries.is_empty() {
            return;
        }
        let Some(source) = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.clone())
        else {
            return;
        };
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };

        for entry in entries {
            let key = entry.key.clone();
            let library = S::library_mut(&mut self.kits[kit_index]);
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

            let (tx, ctx, source) = (self.tx.clone(), ctx.clone(), source.clone());
            thread::spawn(move || {
                // `catch_unwind` because tags have panicked the bitmap decoders
                // and the geometry parser before: a thread that panics never
                // sends, so `pending` would keep its key and one of the four
                // slots would be gone for good. Four such tags stopped the
                // library and every hover preview.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    S::render(&source, &entry, max_edge)
                }))
                .unwrap_or_else(|_| Err(S::CRASHED.to_owned()));
                let _ = tx.send(S::message(stamp, key, result));
                ctx.request_repaint();
            });
        }
    }

    pub(super) fn handle_thumbnail_ready<S: ThumbnailSource>(
        &mut self,
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.resolve_kit(stamp.kit) else {
            // The kit closed while this ran.
            return true;
        };
        // Clear the in-flight marker before deciding whether the result is
        // stale. Returning first, as this did, left it set after any generation
        // bump that landed mid-job, so the work was never asked for again.
        S::library_mut(&mut self.kits[kit_index])
            .pending
            .remove(&key);
        if self.resolve_stamp(stamp).is_none() {
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
        let library = S::library_mut(&mut self.kits[kit_index]);
        if let Ok(mut thumbnails) = library.thumbnails.lock() {
            thumbnails.insert(key, texture);
        }
        false
    }
}

#[cfg(test)]
#[path = "tests/thumbnail_library.rs"]
mod tests;
