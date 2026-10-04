//! The Bitmap Library: every bitmap in a kit as a searchable thumbnail grid.
//! It owns what is particular to bitmaps — which tags are listed, the
//! thumbnail decode, the extract menu item — and the bitmap hover previews
//! that share its cache. The grid itself is `thumbnail_library`'s; bitmap
//! decoding, tag reading, and the tab layout belong elsewhere.

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

#[derive(Clone)]
struct BitmapHoverContext {
    /// The kit's thumbnail cache. Hover previews read it directly rather than
    /// a copy in egui memory: those copies were keyed by kit generation, never
    /// removed, and kept every texture alive past the cache's cap.
    thumbnails: Arc<Mutex<ThumbnailCache>>,
    requests: Arc<Mutex<Vec<TagEntry>>>,
}

fn bitmap_hover_context_id() -> egui::Id {
    egui::Id::new("bitmap_hover_context")
}

/// Start collecting hover-preview requests for the kit about to be drawn.
/// Browser rows and tag-reference fields use the same context and cache.
pub(in crate::app) fn begin_bitmap_hovers(
    ui: &Ui,
    thumbnails: Arc<Mutex<ThumbnailCache>>,
) -> Arc<Mutex<Vec<TagEntry>>> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    ui.data_mut(|data| {
        data.insert_temp(
            bitmap_hover_context_id(),
            BitmapHoverContext {
                thumbnails,
                requests: Arc::clone(&requests),
            },
        )
    });
    requests
}

/// Return a cached hover texture, or enqueue this entry for an asynchronous
/// decode. The outer `Option` distinguishes "not decoded" from a cached miss.
///
/// Call it for a hovered widget only: a request is a tag read and a decode.
pub(in crate::app) fn bitmap_hover_texture(
    ui: &Ui,
    entry: &TagEntry,
) -> Option<Option<egui::TextureHandle>> {
    let context = ui.data(|data| data.get_temp::<BitmapHoverContext>(bitmap_hover_context_id()))?;
    if let Some(cached) = context
        .thumbnails
        .lock()
        .ok()
        .and_then(|mut thumbnails| thumbnails.get(&entry.key))
    {
        return Some(cached);
    }
    if let Ok(mut requests) = context.requests.lock()
        && !requests.iter().any(|request| request.key == entry.key)
    {
        requests.push(entry.clone());
    }
    None
}

/// Painter-only popup shared by browser drag sources and editable reference
/// fields. It registers no tooltip area, so the popup cannot steal a click,
/// text selection, or drop from the widget beneath it.
pub(in crate::app) fn paint_bitmap_hover_preview(
    ui: &Ui,
    response: &egui::Response,
    texture: &egui::TextureHandle,
    text: &str,
) {
    if !response.hovered() || response.dragged() || !hover_popup_due(ui) {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        response.id.with("bitmap_hover_preview"),
    ));
    let image_size = fit_within(texture.size_vec2(), 256.0);
    let galley = painter.layout(
        text.to_owned(),
        FontId::proportional(12.5),
        text_dark(),
        360.0,
    );
    let padding = Vec2::new(7.0, 7.0);
    let gap = 5.0;
    let content_width = image_size.x.max(galley.size().x);
    let content_height = image_size.y + gap + galley.size().y;
    let mut rect = egui::Rect::from_min_size(
        pointer + Vec2::new(14.0, 18.0),
        Vec2::new(content_width, content_height) + padding * 2.0,
    );
    let screen = ui.ctx().content_rect();
    if rect.right() > screen.right() {
        rect = rect.translate(Vec2::new(screen.right() - rect.right(), 0.0));
    }
    if rect.bottom() > screen.bottom() {
        rect = rect.translate(Vec2::new(0.0, -rect.height() - 24.0));
    }
    let visuals = ui.visuals();
    painter.rect(rect, 4.0, visuals.window_fill, visuals.window_stroke, egui::StrokeKind::Middle);
    let image_min = egui::pos2(rect.center().x - image_size.x * 0.5, rect.top() + padding.y);
    painter.image(
        texture.id(),
        egui::Rect::from_min_size(image_min, image_size),
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    painter.galley(
        egui::pos2(rect.left() + padding.x, image_min.y + image_size.y + gap),
        galley,
        text_dark(),
    );
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
    let (rgba, width, height) = crate::app::editor::shader::downscale_rgba(
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

/// The Bitmap Library's [`ThumbnailSource`].
pub(in crate::app) struct Bitmaps;

impl ThumbnailSource for Bitmaps {
    const ID_SALT: &'static str = "bitmap_library";
    const PLURAL: &'static str = "bitmaps";
    const SEARCH_HINT: &'static str = "grass | metal, ^ui, _bump$";
    const INDEXING: &'static str = "Indexing the kit — bitmaps will appear as they are found.";
    const NONE_IN_KIT: &'static str = "No bitmap tags in this workspace.";
    const SINGULAR: &'static str = "bitmap";
    const TEXTURE_PREFIX: &'static str = "bitmap_thumb";
    const MENU_ITEM: &'static str = "Extract bitmap images...";
    const LIBRARY: Library = Library::Bitmaps;
    const CRASHED: &'static str = "bitmap decoder crashed";

    fn library(view: &KitView) -> &ThumbnailLibrary<Self> {
        &view.bitmap_browser
    }

    fn library_mut(view: &mut KitView) -> &mut ThumbnailLibrary<Self> {
        &mut view.bitmap_browser
    }

    fn lists(entry: &TagEntry) -> bool {
        is_bitmap_tag(entry)
    }

    fn caption(_entry: &TagEntry) -> &'static str {
        "Bitmap"
    }

    fn hover_hint(_entry: &TagEntry) -> &'static str {
        "Double-click to open, drag onto a bitmap reference, or right-click to extract"
    }

    fn render(
        source: &TagSource,
        entry: &TagEntry,
        max_edge: u32,
    ) -> Result<ThumbnailImage, String> {
        // `read_entry`, not `read_tag_at_path`: the library covers every
        // source a kit can be, and this is the one reader that knows how each
        // stores its tags — including the JSON layout classic Halo CE and
        // Halo 2 bitmaps need to parse at all.
        crate::core::source::read_entry(source, entry)
            .and_then(|tag| decode_thumbnail(&tag, 0, max_edge))
            .map_err(|error| error.to_string())
    }

    fn message(
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
    ) -> WorkerMessage {
        WorkerMessage::BitmapThumbnailDecoded { stamp, key, result }
    }
}

impl Baboon {
    /// Open (or focus) the Bitmap Library in the active kit.
    pub(in crate::app) fn open_bitmap_library(&mut self) {
        let kit = self.model.active;
        if self.model.kits[kit].source.is_none() {
            self.model.status = "Load an editing kit before browsing its bitmaps".to_owned();
            return;
        }
        self.kit_and_view(kit).open_tag_pane(BITMAP_LIBRARY_KEY);
    }

}

/// Shared contents of the shader-reference hover and any other ordinary egui
/// tooltip that previews a bitmap.
pub(in crate::app) fn bitmap_hover_preview_ui(
    ui: &mut Ui,
    texture: &egui::TextureHandle,
    label: &str,
    label_color: Color32,
) {
    ui.add(egui::Image::new(egui::load::SizedTexture::new(
        texture.id(),
        fit_within(texture.size_vec2(), 256.0),
    )));
    ui.label(RichText::new(label).small().color(label_color));
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
mod tests {
    //! The Bitmap Library's non-drawing halves: which tags it lists, which mip it
    //! decodes, and what its cache throws away.
    //!
    //! The grid itself needs a GPU context to say anything useful, so what is
    //! covered here is the logic that decides how much work the grid does — the
    //! part that turns a kit with twenty thousand bitmaps into something that
    //! scrolls, and the part that would quietly leak GPU memory if it stopped
    //! evicting.

    use super::*;
    use crate::app::kits::{KitMut, tag_tree_id};

    fn entry(display_path: &str, group: &[u8; 4], group_name: Option<&str>) -> TagEntry {
        TagEntry {
            key: format!("file:{display_path}"),
            display_path: display_path.to_owned(),
            group_tag: u32::from_be_bytes(*group),
            group_name: group_name.map(str::to_owned),
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    /// `is_bitmap_tag` is lenient on purpose — group naming varies across the eight
    /// games — so the library must inherit all three of its answers rather than
    /// testing the four-CC alone and quietly missing a game's bitmaps.
    #[test]
    fn every_way_a_bitmap_identifies_itself_is_listed() {
        let by_fourcc = entry("bitmaps/a.bitmap", b"bitm", None);
        let by_group_name = entry("bitmaps/b.bitmap", b"____", Some("bitmap"));
        let by_extension = entry("bitmaps/c.bitmap", b"____", None);
        let not_a_bitmap = entry("shaders/d.shader", b"shad", Some("shader"));

        assert!(is_bitmap_tag(&by_fourcc));
        assert!(is_bitmap_tag(&by_group_name));
        assert!(is_bitmap_tag(&by_extension));
        assert!(!is_bitmap_tag(&not_a_bitmap));
    }

    /// The search box is the tag browser's own filter, so the grammar users already
    /// know works here — including the plain substring the request asked for.
    #[test]
    fn the_search_box_takes_the_browsers_own_filter_grammar() {
        let grass = entry("bitmaps/field_grass.bitmap", b"bitm", Some("bitmap"));
        let metal = entry("bitmaps/metal_plate.bitmap", b"bitm", Some("bitmap"));

        assert!(entry_matches(&grass, "grass"));
        assert!(!entry_matches(&metal, "grass"));
        // OR, so one search can cover two materials.
        assert!(entry_matches(&grass, "grass | metal"));
        assert!(entry_matches(&metal, "grass | metal"));
        // Anchors match the file name, not the folder.
        assert!(entry_matches(&metal, "^metal"));
        assert!(!entry_matches(&metal, "^bitmaps"));
    }

    /// Decoding mip 0 to fill a small cell is the one thing that would make this
    /// grid unusable on a real kit: a 2048px BC7 is sixteen megabytes of RGBA per
    /// thumbnail, thousands of times over.
    #[test]
    fn a_thumbnail_decodes_the_smallest_mip_that_still_covers_the_cell() {
        // 2048 → 1024 → 512 → 256 → 128 …; 192 wants the 256 level, which is 3.
        assert_eq!(smallest_mip_covering(2048, 2048, 12, 192), 3);
        // A cell as big as the bitmap takes level 0 — there is nothing to skip.
        assert_eq!(smallest_mip_covering(256, 256, 9, 256), 0);
        // Non-square: the longest edge decides, so the short one is never starved.
        assert_eq!(smallest_mip_covering(1024, 128, 11, 256), 2);
    }

    #[test]
    fn a_bitmap_with_no_mip_chain_still_decodes_its_only_level() {
        assert_eq!(smallest_mip_covering(2048, 2048, 1, 96), 0);
        assert_eq!(smallest_mip_covering(2048, 2048, 0, 96), 0);
    }

    /// Every level is smaller than the cell — a 32px bitmap in a 96px cell — so the
    /// walk must stop at 0 rather than running to the end of the chain and drawing
    /// a single pixel.
    #[test]
    fn a_bitmap_smaller_than_the_cell_is_not_walked_down_to_nothing() {
        assert_eq!(smallest_mip_covering(32, 32, 6, 96), 0);
    }

    /// Without eviction this cache is the tag editor's unbounded one with extra
    /// steps, and a scroll through a big kit ends in GPU memory exhaustion.
    #[test]
    fn the_thumbnail_cache_evicts_a_batch_once_it_passes_its_cap() {
        let mut cache = ThumbnailCache::default();
        for index in 0..THUMBNAIL_CACHE_CAP {
            cache.insert(format!("key{index}"), None);
        }
        assert_eq!(cache.entries.len(), THUMBNAIL_CACHE_CAP);

        cache.insert("one-too-many".to_owned(), None);
        assert_eq!(
            cache.entries.len(),
            THUMBNAIL_CACHE_CAP + 1 - THUMBNAIL_EVICT_BATCH,
            "passing the cap should drop a batch, not a single entry"
        );
        assert!(
            cache.contains("one-too-many"),
            "the entry that triggered eviction is the newest and must survive"
        );
    }

    /// Recency is what the eviction reads, and drawing a cell is what refreshes it.
    /// An entry the user is still looking at must outlive one they scrolled past.
    #[test]
    fn a_thumbnail_drawn_recently_outlives_one_that_was_not() {
        let mut cache = ThumbnailCache::default();
        cache.insert("old".to_owned(), None);
        for index in 0..THUMBNAIL_CACHE_CAP - 1 {
            cache.insert(format!("key{index}"), None);
        }
        // Touch it, as drawing its cell would.
        assert!(cache.get("old").is_some());
        cache.insert("newest".to_owned(), None);

        assert!(
            cache.contains("old"),
            "a thumbnail touched after insertion must not be evicted as the oldest"
        );
    }

    /// A failed decode is cached as `None` rather than dropped. Retrying an empty
    /// or unsupported bitmap on every frame it stays on screen is the one way this
    /// grid could still stall.
    #[test]
    fn a_failed_decode_is_remembered_so_it_is_not_retried_every_frame() {
        let mut cache = ThumbnailCache::default();
        cache.insert("broken".to_owned(), None);
        assert!(cache.contains("broken"));
        assert!(matches!(cache.get("broken"), Some(None)));
    }

    #[test]
    fn a_cell_shows_the_tag_name_without_its_folder_or_group() {
        assert_eq!(
            tag_leaf_name("bitmaps\\ui\\field_grass.bitmap"),
            "field_grass"
        );
        assert_eq!(tag_leaf_name("bitmaps/field_grass.bitmap"), "field_grass");
        assert_eq!(tag_leaf_name("field_grass"), "field_grass");
    }

    /// A tall or wide bitmap keeps its shape in a square cell, and a small one is
    /// not blown up past its own resolution.
    #[test]
    fn a_thumbnail_fits_its_cell_without_being_stretched_or_enlarged() {
        let wide = fit_within(Vec2::new(256.0, 64.0), 96.0);
        assert_eq!(wide, Vec2::new(96.0, 24.0));

        let small = fit_within(Vec2::new(32.0, 32.0), 96.0);
        assert_eq!(small, Vec2::new(32.0, 32.0), "never scaled up");
    }

    /// The width every column actually occupies, laid out end to end.
    ///
    /// `n` cells and the `n - 1` gaps between them — no trailing gap, because the
    /// row's `item_spacing` puts the gap *between* cells only.
    fn row_width(columns: usize, cell: f32) -> f32 {
        columns as f32 * cell + (columns.saturating_sub(1)) as f32 * CELL_GAP
    }

    /// The rightmost thumbnail was drawn half off the edge of the pane.
    ///
    /// Two errors compounded: the trailing gap after the last cell was counted as
    /// usable space, and egui's own `item_spacing` was added between cells on top
    /// of the gap. Both round the column count up, and on a wide window — 2560px at
    /// default scale, where a dozen columns fit — the accumulated overflow is most
    /// of a cell.
    ///
    /// The invariant is simply that what is drawn fits in what was measured, so
    /// that is what this asserts, across the whole slider range and a spread of
    /// realistic pane widths.
    #[test]
    fn the_rightmost_column_always_fits_inside_the_pane() {
        for usable in [
            320.0, 640.0, 800.0, 1024.0, 1280.0, 1600.0, 1920.0, 2304.0, 2560.0, 3440.0,
        ] {
            for cell in [MIN_CELL, 64.0, DEFAULT_CELL, 128.0, 192.0, MAX_CELL] {
                let columns = grid_columns(usable, cell);
                assert!(
                    row_width(columns, cell) <= usable,
                    "{columns} cells of {cell}px overflow a {usable}px pane by {}px",
                    row_width(columns, cell) - usable
                );
                // And it must not be needlessly conservative either — one more
                // column has to genuinely not fit, or the grid wastes a column.
                assert!(
                    row_width(columns + 1, cell) > usable,
                    "another {cell}px cell would still have fitted in {usable}px"
                );
            }
        }
    }

    /// A pane narrower than one thumbnail still shows one, clipped, rather than
    /// dividing by zero into an empty grid.
    #[test]
    fn a_pane_too_narrow_for_one_thumbnail_still_shows_one() {
        assert_eq!(grid_columns(10.0, MAX_CELL), 1);
        assert_eq!(grid_columns(0.0, DEFAULT_CELL), 1);
    }

    /// Exactly-fitting widths are where an off-by-one hides: `n` cells plus their
    /// gaps landing precisely on the pane edge must count as fitting.
    #[test]
    fn a_row_that_fits_exactly_counts_as_fitting() {
        for columns in [1usize, 2, 5, 12] {
            let exact = row_width(columns, DEFAULT_CELL);
            assert_eq!(
                grid_columns(exact, DEFAULT_CELL),
                columns,
                "{columns} cells filling {exact}px exactly"
            );
            // One point short, and the last one has to be given up.
            assert_eq!(
                grid_columns(exact - 1.0, DEFAULT_CELL),
                columns.saturating_sub(1).max(1)
            );
        }
    }

    /// Double-clicking a thumbnail loaded the tag and never showed a tab.
    ///
    /// The grid draws inside `tree.ui`, and `draw_tag_tiles` has moved the kit's
    /// `tag_tree` out for the duration — so opening a pane from in there writes it
    /// into the placeholder that is thrown away when the real tree goes back. The
    /// tag loaded, which is why the status line said so, and the tab never
    /// appeared. A cell's open is therefore a command, which runs once drawing is
    /// over and the tree is back.
    #[test]
    fn opening_a_bitmap_must_wait_until_the_tag_tree_is_back() {
        const KEY: &str = "file:bitmaps/field_grass";

        // What the draw does: take the tree, walk it, put it back.
        let mut kit = Kit::empty(KitId(1), TagNameIndex::default());
        let mut view = KitView::for_test(&kit);
        let taken = std::mem::replace(
            &mut view.tag_tree,
            egui_tiles::Tree::empty(tag_tree_id(kit.id)),
        );

        // The bug: opening during the walk lands in the placeholder.
        KitMut::new(&mut kit, &mut view).open_tag_pane(KEY);
        view.tag_tree = taken;
        KitMut::new(&mut kit, &mut view).sync_open_tabs();
        assert!(
            kit.open_tabs.is_empty(),
            "a pane opened during the walk cannot survive the tree being restored"
        );

        // The fix: the cell sends its open during the walk, and the frame applies
        // it once the tree is back.
        let mut app = Baboon::for_test();
        let kit = app.model.kits[0].id;
        let taken = std::mem::replace(
            &mut app.views[kit].tag_tree,
            egui_tiles::Tree::empty(tag_tree_id(kit)),
        );
        app.commands.send(BrowserCommand::LibraryCell {
            kit,
            library: Library::Bitmaps,
            action: CellAction::Open(KEY.to_owned()),
        });
        app.views[kit].tag_tree = taken;
        app.apply_commands(&egui::Context::default());

        assert_eq!(app.model.kits[0].open_tabs, vec![KEY.to_owned()]);
        assert_eq!(app.model.kits[0].selected_key.as_deref(), Some(KEY));
    }

    /// The session writer decides the Bitmap Library was open by looking for its
    /// pane key in `open_tabs`, so that key has to actually land there.
    ///
    /// `open_tabs` is derived from the tile tree rather than written to directly,
    /// and the library's key resolves to no tag — the one place that could have
    /// filtered it back out.
    #[test]
    fn an_open_bitmap_library_shows_up_in_the_kits_open_tabs() {
        let mut kit = Kit::empty(KitId(1), TagNameIndex::default());
        let mut view = KitView::for_test(&kit);
        assert!(!kit.open_tabs.iter().any(|key| key == BITMAP_LIBRARY_KEY));

        KitMut::new(&mut kit, &mut view).open_tag_pane(BITMAP_LIBRARY_KEY);
        assert!(
            kit.open_tabs.iter().any(|key| key == BITMAP_LIBRARY_KEY),
            "the session writer looks for exactly this: {:?}",
            kit.open_tabs
        );

        // And closing it takes the flag away again, so a library the user shut
        // does not reopen next launch.
        KitMut::new(&mut kit, &mut view).close_tag_pane(BITMAP_LIBRARY_KEY);
        assert!(!kit.open_tabs.iter().any(|key| key == BITMAP_LIBRARY_KEY));
    }

    /// Right-click → Extract has to resolve a bitmap the grid listed, and the grid
    /// lists the *whole* kit.
    ///
    /// The library reads `full_entry_set`, which on a loose kit is the background
    /// index (`all_entries`) rather than the folders the browser has expanded
    /// (`entries`). Extraction goes the other way round, through `entry_for_key`.
    /// If that looked at `entries` alone, extracting would silently do nothing for
    /// almost every bitmap in the grid — the browser's own bulk extract does read
    /// `entries` only, so the two really do differ.
    #[test]
    fn extraction_resolves_a_bitmap_the_grid_listed_from_the_background_index() {
        let indexed = entry("bitmaps/deep/field_grass.bitmap", b"bitm", Some("bitmap"));
        let mut kit = Kit::empty(KitId(1), TagNameIndex::default());
        kit.source = Some(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("C:/kit/tags"),
                game: Some(GameId::Halo3),
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: Some(GameId::Halo3),
            // Not expanded in the browser, so absent here …
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            // … but found by the background scan, which is what the grid shows.
            all_entries: vec![indexed.clone()],
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });

        let source = kit.source.as_ref().unwrap();
        assert!(
            source
                .full_entry_set()
                .iter()
                .any(|entry| entry.key == indexed.key),
            "the grid lists it"
        );
        assert_eq!(
            kit.entry_for_key(&indexed.key).map(|entry| &entry.key),
            Some(&indexed.key),
            "so extraction has to find it too"
        );
    }

    /// Dragging a thumbnail onto a shader's bitmap slot works because the cell
    /// emits the browser row's own payload, which that slot already accepts.
    ///
    /// The two ends are in different modules and neither imports the other, so the
    /// only thing holding them together is the payload's shape — and the group test
    /// the shader row does on it, reproduced here verbatim.
    #[test]
    fn a_dragged_thumbnail_carries_what_a_shader_bitmap_slot_expects() {
        let bitmap = entry(
            "scenery\\rock\\rock_diffuse.bitmap",
            b"bitm",
            Some("bitmap"),
        );

        let payload = DraggedTagRef {
            group_tag: bitmap.group_tag,
            input: entry_reference_input(&bitmap),
            rel_path: entry_rel_path(&bitmap),
            file_path: entry_loose_file(&bitmap),
        };

        // `src/app/shader/editing/mod.rs` gates the drop on exactly this.
        assert_eq!(&payload.group_tag.to_be_bytes(), b"bitm");
        // The shader row writes `rel_path` straight into the field: forward slashes,
        // no group extension.
        assert_eq!(payload.rel_path, "scenery/rock/rock_diffuse");
        // Foundation reference cells take `input` instead, in four-CC form.
        assert!(
            payload.input.starts_with("bitm:"),
            "reference cells parse the group prefix: {}",
            payload.input
        );
        assert!(
            !payload.input.contains(".bitmap"),
            "the group extension is carried by the four-CC, not the path: {}",
            payload.input
        );
    }

    /// Point this at an editing kit's `tags` folder to exercise the decode against
    /// real bitmaps. Absent, the fixture test below self-skips like the rest.
    const KIT_TAGS_ENV: &str = "BABOON_BITMAP_KIT";

    /// Decode real bitmaps out of a real kit, at the size a grid cell asks for.
    ///
    /// The unit tests above pin the arithmetic; this pins the thing the arithmetic
    /// is for. A mip index that is right on paper and wrong against a shipped tag —
    /// off the end of the chain, or naming a level whose bytes are not there — is
    /// the failure this catches, and it can only be caught against real data.
    #[test]
    fn real_kit_bitmaps_decode_at_thumbnail_size() {
        let Some(tags_root) = std::env::var_os(KIT_TAGS_ENV).map(PathBuf::from) else {
            eprintln!("skipping: set {KIT_TAGS_ENV} to an editing kit's tags folder");
            return;
        };
        if !tags_root.is_dir() {
            eprintln!("skipping: {} is not a folder", tags_root.display());
            return;
        }

        let bitmaps: Vec<PathBuf> = walkdir::WalkDir::new(&tags_root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|found| {
                found.file_type().is_file()
                    && found
                        .path()
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("bitmap"))
            })
            .map(|found| found.path().to_path_buf())
            .take(40)
            .collect();
        if bitmaps.is_empty() {
            eprintln!("skipping: no .bitmap tags under {}", tags_root.display());
            return;
        }

        const CELL: u32 = 192;
        let mut decoded = 0;
        for path in &bitmaps {
            let group = u32::from_be_bytes(*b"bitm");
            let Ok(tag) = crate::core::source::read_tag_at_path(path, None, None, group) else {
                continue;
            };
            // Some shipped bitmaps genuinely hold no images; that is a `None`
            // thumbnail in the grid, not a failure here.
            let Ok(image) = decode_thumbnail(&tag, 0, CELL) else {
                continue;
            };
            assert!(
                image.width > 0 && image.height > 0,
                "{} decoded to an empty image",
                path.display()
            );
            assert_eq!(
                image.rgba.len(),
                image.width * image.height * 4,
                "{} produced a buffer that is not tightly packed RGBA8",
                path.display()
            );
            assert!(
                image.width <= CELL as usize && image.height <= CELL as usize,
                "{} came back at {}x{}, larger than the {CELL}px cell asked for",
                path.display(),
                image.width,
                image.height
            );
            decoded += 1;
        }
        assert!(
            decoded > 0,
            "{} bitmaps found under {} and none decoded",
            bitmaps.len(),
            tags_root.display()
        );
        eprintln!("decoded {decoded} of {} real bitmaps", bitmaps.len());
    }

    /// The pane key must be one no tag can produce, or the library would collide
    /// with a real tag's tab and the entry lookup would resolve it.
    #[test]
    fn the_library_pane_key_cannot_collide_with_a_tag_key() {
        assert!(BITMAP_LIBRARY_KEY.starts_with("tool:"));
        for prefix in ["file:", "cache:", "ublock:"] {
            assert!(!BITMAP_LIBRARY_KEY.starts_with(prefix));
        }
    }
}

#[cfg(test)]
mod stale_result_tests {
    use super::*;

    /// A thumbnail that lands after a generation bump is dropped, but its key
    /// must leave `pending`: it used to stay, and four such keys stopped every
    /// further decode for the kit.
    #[test]
    fn a_stale_thumbnail_still_frees_its_decode_slot() {
        let mut app = Baboon::for_test();
        let stamp = app.model.kit_stamp();
        app.views[app.model.kits[0].id]
            .bitmap_browser
            .pending
            .insert("file:a.bitmap".to_owned());
        app.model.kits[0].generation = app.model.kits[0].generation.wrapping_add(1);

        app.handle_thumbnail_ready::<Bitmaps>(
            stamp,
            "file:a.bitmap".to_owned(),
            Err("stale".to_owned()),
            &egui::Context::default(),
        );

        assert!(app.views[app.model.kits[0].id].bitmap_browser.pending.is_empty());
        let cached = app.views[app.model.kits[0].id]
            .bitmap_browser
            .thumbnails
            .lock()
            .unwrap()
            .contains("file:a.bitmap");
        assert!(!cached, "the stale result itself is not kept");
    }

    /// A generation bump keeps the thumbnails that are still right: listed
    /// and unmodified. Everything used to be thrown away and decoded again.
    #[test]
    fn a_generation_bump_keeps_thumbnails_that_are_still_right() {
        let root = std::env::temp_dir().join(format!(
            "baboon-thumb-revalidate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let set_time = |path: &Path, seconds: u64| {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap();
        };
        let (same, changed, gone) = (
            root.join("same.bitmap"),
            root.join("changed.bitmap"),
            root.join("gone.bitmap"),
        );
        for path in [&same, &changed, &gone] {
            std::fs::write(path, b"bitmap").unwrap();
            set_time(path, 1_000_000);
        }
        let key = |path: &Path| file_entry_key(&path);
        let mut cache = ThumbnailCache::default();
        for listed in [
            key(&same),
            key(&changed),
            key(&gone),
            "ublock:0:pak.bitmap".to_owned(),
        ] {
            cache.insert(listed, None);
        }
        set_time(&changed, 2_000_000);

        let listed = [key(&same), key(&changed), "ublock:0:pak.bitmap".to_owned()];
        cache.revalidate(|candidate| listed.iter().any(|key| key == candidate));

        std::fs::remove_dir_all(&root).unwrap();
        assert!(cache.contains(&key(&same)), "unchanged: kept");
        assert!(
            !cache.contains(&key(&changed)),
            "modified since it was decoded: dropped"
        );
        assert!(!cache.contains(&key(&gone)), "no longer listed: dropped");
        assert!(
            cache.contains("ublock:0:pak.bitmap"),
            "a pak tag cannot change: kept"
        );
    }

    /// Same for a model preview's texture resolve: a stale result left
    /// `textures_pending` set, and the preview showed "Loading shaders…" and
    /// repainted every frame for good.
    #[test]
    fn a_stale_texture_resolve_clears_textures_pending() {
        let mut app = Baboon::for_test();
        let stamp = app.model.kit_stamp();
        let state = app.views[app.model.kits[0].id]
            .caches.model_previews
            .entry("file:a.model".to_owned())
            .or_default();
        state.textures_pending = true;
        app.model.kits[0].generation = app.model.kits[0].generation.wrapping_add(1);

        app.handle_model_textures_resolved(stamp, "file:a.model".to_owned(), 1, Vec::new());

        assert!(!app.views[app.model.kits[0].id].caches.model_previews["file:a.model"].textures_pending);
    }
}

/// Start the thumbnails the bitmap hovers drawn this frame asked for.
pub(in crate::app) fn queue_bitmap_hover_thumbnails(
    cx: &Ctx,
    kit_index: usize,
    library: &mut ThumbnailLibrary<Bitmaps>,
    requests: &Arc<Mutex<Vec<TagEntry>>>,
) {
    let entries = requests
        .lock()
        .map(|mut requests| std::mem::take(&mut *requests))
        .unwrap_or_default();
    queue_thumbnails::<Bitmaps>(cx, kit_index, library, entries, 256);
}
