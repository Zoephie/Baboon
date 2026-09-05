//! The Bitmap Library: every bitmap in a kit as a searchable thumbnail grid.
//! It owns the grid's state, its bounded thumbnail cache, and its presentation;
//! bitmap decoding, tag reading, and the tab layout belong elsewhere. The cache,
//! cell metrics, and grid arithmetic are shared with the Model Library
//! (`model_browser`), which lays out the same grid over different thumbnails.

use super::*;

/// The pane key the Bitmap Library occupies.
///
/// `Kit::tag_tree` panes are tag keys, so a tab that is not a tag needs a key no
/// tag can have. The `tool:` prefix follows the namespacing the other synthetic
/// keys already use — `cache:{group}:{name}`, `ublock:{chunk}:{path}` — and
/// nothing resolves it against the source, which is what makes it work: the
/// pane has no document, so the close path finds nothing dirty and the session
/// writer skips it rather than trying to reopen a tag that does not exist.
pub(in crate::app) const BITMAP_LIBRARY_KEY: &str = "tool:bitmap_library";

pub(in crate::app) const BITMAP_LIBRARY_TITLE: &str = "Bitmap Library";

/// How many decoded thumbnails are held at once.
///
/// Each is a GPU texture, so this cannot be the unbounded map the tag editor's
/// `bitmap_previews` is: that one holds a full-resolution RGBA buffer plus a
/// texture per bitmap the user opened, which is fine for a handful of tabs and
/// ruinous across a kit with thousands of bitmaps. Dropping the handle frees
/// the texture.
const THUMBNAIL_CACHE_CAP: usize = 512;

/// How many are dropped once the cap is passed.
///
/// A batch, rather than one per insert: eviction scans the cache for the least
/// recently drawn entries, and doing that on every new thumbnail while the user
/// scrolls would be the most expensive thing in the frame.
const THUMBNAIL_EVICT_BATCH: usize = 128;

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

/// One kit's Bitmap Library.
#[derive(Default)]
pub(in crate::app) struct BitmapBrowserState {
    pub(in crate::app) filter: String,
    pub(in crate::app) cell_size: f32,
    /// Every bitmap in the kit, snapshotted rather than re-scanned per frame.
    entries: Vec<TagEntry>,
    /// The kit generation `entries` was taken at, so a reload refreshes it.
    entries_for: Option<u64>,
    /// Indices into `entries` matching `filter`.
    matches: Vec<usize>,
    /// The query `matches` was computed for. Filtering tens of thousands of
    /// entries is cheap once and wasteful sixty times a second.
    matched_for: Option<String>,
    thumbnails: ThumbnailCache,
    /// Keys with a decode job running, so a cell is not queued twice while its
    /// thread works.
    pending: HashSet<String>,
    /// Set once the "scan the whole kit" request has gone out, so the browser
    /// does not ask again every frame it is drawn.
    requested_scan: bool,
    /// A double-clicked bitmap waiting to be opened as a tab.
    ///
    /// The grid draws inside `tree.ui`, where the kit's `tag_tree` has been
    /// moved out; opening there writes the tab into a placeholder that is
    /// discarded. `draw_tag_tiles` takes this once the tree is back.
    pub(in crate::app) pending_open: Option<String>,
    /// A bitmap whose right-click menu asked to extract its images.
    ///
    /// Parked for the same reason, and one more: extraction opens a native
    /// folder picker, which blocks the thread until the user answers it. Doing
    /// that mid-walk would stall the frame with the kit's tag tree still moved
    /// out of it.
    pub(in crate::app) pending_extract: Option<String>,
}

impl BitmapBrowserState {
    fn cell_size(&self) -> f32 {
        if self.cell_size <= 0.0 {
            DEFAULT_CELL
        } else {
            self.cell_size.clamp(MIN_CELL, MAX_CELL)
        }
    }
}

/// A bounded, least-recently-drawn thumbnail cache.
///
/// `None` is cached as well as `Some`: a bitmap that fails to decode — an empty
/// tag, an unsupported format — must not be retried on every frame it is
/// visible, which is what made the shader editor's inline thumbnails cache
/// their failures too.
#[derive(Default)]
pub(in crate::app) struct ThumbnailCache {
    entries: HashMap<String, Thumbnail>,
    /// Monotonic draw counter; `Thumbnail::used` is a stamp from it.
    clock: u64,
}

struct Thumbnail {
    texture: Option<egui::TextureHandle>,
    used: u64,
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
        self.entries.insert(key, Thumbnail { texture, used });
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

    pub(in crate::app) fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A bitmap decoded small enough to draw in a grid cell.
pub(in crate::app) struct ThumbnailImage {
    pub(in crate::app) rgba: Vec<u8>,
    pub(in crate::app) width: usize,
    pub(in crate::app) height: usize,
}

/// Decode one bitmap down to something a `max_edge`-point cell can draw.
///
/// The mip chain does most of the work: a 2048×2048 BC7 decoded at level 0 to
/// fill a 96-point cell is sixteen megabytes of RGBA thrown away, and a kit has
/// thousands of these. Pick the smallest level that still covers the cell and
/// decode only that; `downscale_rgba` takes it the rest of the way, and covers
/// the bitmaps that ship no mips at all.
pub(in crate::app) fn decode_thumbnail(
    tag: &TagFile,
    image_index: usize,
    max_edge: u32,
) -> anyhow::Result<ThumbnailImage> {
    let bitmap = Bitmap::new(tag)?;
    if bitmap.is_empty() {
        anyhow::bail!("bitmap tag has no images");
    }
    // Clamped rather than rejected: a shader may name an image index the
    // bitmap it points at no longer has, and one stale index should cost the
    // wrong image rather than the whole material.
    let image_index = image_index.min(bitmap.len() - 1);
    let image = bitmap
        .image(image_index)
        .ok_or_else(|| anyhow::anyhow!("bitmap tag has no image {image_index}"))?;
    let mip = smallest_mip_covering(
        image.width(),
        image.height(),
        (image.mipmap_levels() as usize).max(1),
        max_edge,
    );
    let data = build_bitmap_preview(tag, image_index, mip)?;
    let (rgba, width, height) = crate::app::shader::downscale_rgba(
        &data.rgba,
        data.width as u32,
        data.height as u32,
        max_edge,
    );
    if width == 0 || height == 0 {
        anyhow::bail!("bitmap image is empty at every mip level");
    }
    Ok(ThumbnailImage {
        rgba,
        width,
        height,
    })
}

/// What a grid cell asked for this frame.
///
/// Both are parked rather than run on the spot — see the fields they land in on
/// [`BitmapBrowserState`].
enum CellAction {
    Open(String),
    Extract(String),
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

/// The deepest mip level whose longest edge is still at least `max_edge`.
///
/// Never the last level for its own sake: going below the cell size trades a
/// blurry thumbnail for a decode that was already cheap.
fn smallest_mip_covering(width: u32, height: u32, mip_count: usize, max_edge: u32) -> usize {
    let mut level = 0;
    let (mut w, mut h) = (width, height);
    while level + 1 < mip_count {
        let (next_w, next_h) = ((w / 2).max(1), (h / 2).max(1));
        if next_w.max(next_h) < max_edge {
            break;
        }
        w = next_w;
        h = next_h;
        level += 1;
    }
    level
}

impl Baboon {
    /// Open (or focus) the Bitmap Library in the active kit.
    pub(super) fn open_bitmap_library(&mut self) {
        let kit = self.active;
        if self.kits[kit].source.is_none() {
            self.status = "Load an editing kit before browsing its bitmaps".to_owned();
            return;
        }
        self.kits[kit].open_tag_pane(BITMAP_LIBRARY_KEY);
    }

    /// Draw one kit's Bitmap Library pane.
    pub(super) fn draw_bitmap_library(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
    ) {
        self.refresh_bitmap_library(kit_index, ctx);

        let cell = self.kits[kit_index].bitmap_browser.cell_size();
        let total = self.kits[kit_index].bitmap_browser.matches.len();
        let all = self.kits[kit_index].bitmap_browser.entries.len();
        let scanning = self.kits[kit_index].scanning_entries;

        self.draw_bitmap_library_toolbar(ui, kit_index, total, all, scanning);
        ui.separator();

        if all == 0 {
            ui.add_space(12.0);
            ui.label(
                RichText::new(if scanning {
                    "Indexing the kit — bitmaps will appear as they are found."
                } else {
                    "No bitmap tags in this workspace."
                })
                .color(subtle_dark()),
            );
            return;
        }
        if total == 0 {
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!(
                    "No bitmap matches that search. {all} in this workspace."
                ))
                .color(subtle_dark()),
            );
            return;
        }

        self.draw_bitmap_grid(ui, ctx, kit_index, cell, total);
    }

    fn draw_bitmap_library_toolbar(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        shown: usize,
        total: usize,
        scanning: bool,
    ) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Search").color(subtle_dark()));
            let browser = &mut self.kits[kit_index].bitmap_browser;
            ui.add(
                egui::TextEdit::singleline(&mut browser.filter)
                    .hint_text(placeholder_text("grass | metal, ^ui, _bump$"))
                    .desired_width(240.0),
            )
            .on_hover_text(
                "Space is AND, | is OR, ^foo and foo$ anchor to the start and end of the name — \
                 the same search the tag browser uses.",
            );
            if ui.button("Clear").clicked() {
                browser.filter.clear();
            }

            ui.separator();
            ui.label(RichText::new("Size").color(subtle_dark()));
            let mut cell = browser.cell_size();
            if ui
                .add(
                    egui::Slider::new(&mut cell, MIN_CELL..=MAX_CELL)
                        .show_value(false)
                        .clamping(egui::SliderClamping::Always),
                )
                .changed()
            {
                browser.cell_size = cell;
            }
            if ui.button("Reset").clicked() {
                browser.cell_size = DEFAULT_CELL;
            }

            ui.separator();
            let count = if shown == total {
                format!("{total} bitmaps")
            } else {
                format!("{shown} of {total} bitmaps")
            };
            ui.label(RichText::new(count).color(subtle_dark()));
            if scanning {
                ui.spinner();
                ui.label(RichText::new("indexing…").color(subtle_dark()));
            }
        });
    }

    fn draw_bitmap_grid(
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
        // lays out the thirty on screen and queues decodes for those alone.
        let mut action: Option<CellAction> = None;
        let mut wanted: Vec<String> = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt(("bitmap_library", kit_index))
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
                            if let Some(requested) =
                                self.draw_bitmap_cell(ui, kit_index, index, cell, &mut wanted)
                            {
                                action = Some(requested);
                            }
                        }
                    });
                    ui.add_space(CELL_GAP);
                }
            });

        self.queue_bitmap_thumbnails(kit_index, wanted, cell, ctx);
        // Parked rather than opened here. This runs inside `tree.ui`, and
        // `draw_tag_tiles` has moved the kit's `tag_tree` out for the duration —
        // so `open_tag_pane` would insert the new tab into the placeholder that
        // is thrown away when the real tree is put back. The tag loaded and no
        // tab ever appeared. `draw_tag_tiles` drains this after the walk, which
        // is where every other pane mutation is applied for the same reason.
        match action {
            Some(CellAction::Open(key)) => {
                self.kits[kit_index].bitmap_browser.pending_open = Some(key)
            }
            Some(CellAction::Extract(key)) => {
                self.kits[kit_index].bitmap_browser.pending_extract = Some(key)
            }
            None => {}
        }
    }

    /// One grid cell, and whatever the user asked it for.
    fn draw_bitmap_cell(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        index: usize,
        cell: f32,
        wanted: &mut Vec<String>,
    ) -> Option<CellAction> {
        let browser = &mut self.kits[kit_index].bitmap_browser;
        let entry_index = *browser.matches.get(index)?;
        let entry = browser.entries.get(entry_index)?;
        let (key, display_path) = (entry.key.clone(), entry.display_path.clone());

        let texture = match browser.thumbnails.get(&key) {
            Some(texture) => texture,
            None => {
                // Not decoded yet. Ask for it, draw the placeholder, and let the
                // worker's reply repaint the frame.
                if !browser.pending.contains(&key) {
                    wanted.push(key.clone());
                }
                None
            }
        };

        let (group_tag, reference_input, rel_path) = (
            entry.group_tag,
            entry_reference_input(entry),
            entry_rel_path(entry),
        );

        let size = Vec2::new(cell, cell + CELL_CAPTION);
        // `click_and_drag`, so a cell is both a target to open and a source to
        // drag. The payload is the browser row's own `DraggedTagRef` — the
        // shader bitmap rows and Foundation reference cells already accept it,
        // and a shader slot already checks for the `bitm` group — so dragging a
        // thumbnail onto a shader's bitmap slot needs nothing on the drop side.
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        response.dnd_set_drag_payload(DraggedTagRef {
            group_tag,
            input: reference_input,
            rel_path,
            file_path: entry_loose_file(entry),
        });
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
                ui.painter().text(
                    image_rect.center(),
                    Align2::CENTER_CENTER,
                    "…",
                    FontId::proportional(14.0),
                    subtle_dark(),
                );
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
            "Bitmap",
            FontId::proportional(10.0),
            subtle_dark(),
        );
        // No trailing space: the row's `item_spacing` puts the gap *between*
        // cells and none after the last one, which is what the column count
        // above assumes. Adding it here too would overflow the row by one gap
        // per cell and push the rightmost column off the edge.

        // The name follows the cursor while dragging, as the browser rows do —
        // the thumbnail is left behind, so without this there is nothing to say
        // which bitmap is in flight.
        if response.dragged()
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            egui::Area::new(ui.make_persistent_id(("bitmap_library_drag_preview", &key)))
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
            if context_menu_button(ui, "Extract bitmap images...").clicked() {
                action = Some(CellAction::Extract(key.clone()));
                ui.close_menu();
            }
        });

        // Suppressed while that menu is open: a tooltip would otherwise sit over
        // the item the cursor is on. Not `on_hover_text`: an egui tooltip would
        // block the drag this cell offers (`hover_tooltip_beside_pointer`).
        if !response.context_menu_opened() {
            hover_tooltip_beside_pointer(
                ui,
                &response,
                &format!(
                    "{display_path}\n\nDouble-click to open, drag onto a bitmap reference, \
                     or right-click to extract"
                ),
            );
        }
        action
    }

    /// Snapshot the kit's bitmaps and recompute the filter, both only when
    /// something they depend on has actually changed.
    fn refresh_bitmap_library(&mut self, kit_index: usize, ctx: &egui::Context) {
        let generation = self.kits[kit_index].generation;
        let stale = self.kits[kit_index].bitmap_browser.entries_for != Some(generation);
        if stale {
            let entries: Vec<TagEntry> = self.kits[kit_index]
                .source
                .as_ref()
                .map(|source| source.full_entry_set())
                .unwrap_or_default()
                .iter()
                .filter(|entry| is_bitmap_tag(entry))
                .cloned()
                .collect();
            let browser = &mut self.kits[kit_index].bitmap_browser;
            browser.entries = entries;
            browser.entries_for = Some(generation);
            browser.matched_for = None;
            browser.thumbnails.clear();
            // A new source gets to ask for its own scan; the flag only exists
            // to stop the request repeating every frame within one source.
            browser.requested_scan = false;
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
            && !self.kits[kit_index].bitmap_browser.requested_scan;
        if needs_scan {
            self.kits[kit_index].bitmap_browser.requested_scan = true;
            self.begin_scan_all_entries(ctx.clone());
        }

        let browser = &mut self.kits[kit_index].bitmap_browser;
        if browser.matched_for.as_deref() != Some(browser.filter.as_str()) {
            let filter = browser.filter.trim().to_owned();
            browser.matches = browser
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| filter.is_empty() || entry_matches(entry, &filter))
                .map(|(index, _)| index)
                .collect();
            browser.matched_for = Some(browser.filter.clone());
        }
    }

    /// Start decode jobs for the visible cells that have none, up to the
    /// in-flight bound.
    fn queue_bitmap_thumbnails(
        &mut self,
        kit_index: usize,
        wanted: Vec<String>,
        cell: f32,
        ctx: &egui::Context,
    ) {
        if wanted.is_empty() {
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
        // Requested at twice the cell's point size, so the thumbnail still looks
        // right after the slider grows a little and on a high-DPI display.
        let max_edge = ((cell * 2.0).round() as u32).max(MIN_CELL as u32);

        for key in wanted {
            if self.kits[kit_index].bitmap_browser.pending.len() >= MAX_DECODES_IN_FLIGHT {
                break;
            }
            if self.kits[kit_index].bitmap_browser.pending.contains(&key)
                || self.kits[kit_index]
                    .bitmap_browser
                    .thumbnails
                    .contains(&key)
            {
                continue;
            }
            let Some(entry) = self.kits[kit_index]
                .bitmap_browser
                .entries
                .iter()
                .find(|entry| entry.key == key)
                .cloned()
            else {
                continue;
            };
            self.kits[kit_index]
                .bitmap_browser
                .pending
                .insert(key.clone());

            let (tx, ctx, source) = (self.tx.clone(), ctx.clone(), source.clone());
            thread::spawn(move || {
                // `read_entry`, not `read_tag_at_path`: the library covers every
                // source a kit can be, and this is the one reader that knows
                // how each stores its tags — including the JSON layout classic
                // Halo CE and Halo 2 bitmaps need to parse at all.
                let result = crate::source::read_entry(&source, &entry)
                    .and_then(|tag| decode_thumbnail(&tag, 0, max_edge))
                    .map_err(|error| error.to_string());
                let _ = tx.send(WorkerMessage::BitmapThumbnailDecoded { stamp, key, result });
                ctx.request_repaint();
            });
        }
    }

    pub(super) fn handle_bitmap_thumbnail_decoded(
        &mut self,
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            // The kit closed or was reloaded while this decoded; its thumbnail
            // belongs to a source that is no longer there.
            return true;
        };
        let browser = &mut self.kits[kit_index].bitmap_browser;
        browser.pending.remove(&key);
        // A failure is cached as `None` rather than dropped: an empty or
        // unsupported bitmap would otherwise be re-decoded every frame it stays
        // on screen, which is the one way this grid could still stall.
        let texture = match result {
            Ok(image) => Some(ctx.load_texture(
                format!("bitmap_thumb:{key}"),
                egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba),
                egui::TextureOptions::LINEAR,
            )),
            Err(_) => None,
        };
        browser.thumbnails.insert(key, texture);
        false
    }
}

/// Scale `size` down to fit a square of `edge` points, never up.
pub(in crate::app) fn fit_within(size: Vec2, edge: f32) -> Vec2 {
    let longest = size.x.max(size.y);
    if longest <= 0.0 {
        return Vec2::splat(edge);
    }
    let scale = (edge / longest).min(1.0);
    size * scale
}

/// The file name a display path ends in, without its group extension.
pub(in crate::app) fn tag_leaf_name(display_path: &str) -> String {
    let leaf = display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(display_path);
    leaf.rsplit_once('.')
        .map_or(leaf, |(stem, _)| stem)
        .to_owned()
}

#[cfg(test)]
#[path = "tests/bitmap_browser.rs"]
mod tests;
