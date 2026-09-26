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
    if !response.hovered() || response.dragged() {
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
    let screen = ui.ctx().screen_rect();
    if rect.right() > screen.right() {
        rect = rect.translate(Vec2::new(screen.right() - rect.right(), 0.0));
    }
    if rect.bottom() > screen.bottom() {
        rect = rect.translate(Vec2::new(0.0, -rect.height() - 24.0));
    }
    let visuals = ui.visuals();
    painter.rect(rect, 4.0, visuals.window_fill, visuals.window_stroke);
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
    const CRASHED: &'static str = "bitmap decoder crashed";

    fn library(kit: &Kit) -> &ThumbnailLibrary<Self> {
        &kit.bitmap_browser
    }

    fn library_mut(kit: &mut Kit) -> &mut ThumbnailLibrary<Self> {
        &mut kit.bitmap_browser
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
        crate::source::read_entry(source, entry)
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
    pub(super) fn open_bitmap_library(&mut self) {
        let kit = self.active;
        if self.kits[kit].source.is_none() {
            self.status = "Load an editing kit before browsing its bitmaps".to_owned();
            return;
        }
        self.kits[kit].open_tag_pane(BITMAP_LIBRARY_KEY);
    }

    pub(super) fn queue_bitmap_hover_thumbnails(
        &mut self,
        kit_index: usize,
        requests: &Arc<Mutex<Vec<TagEntry>>>,
        ctx: &egui::Context,
    ) {
        let entries = requests
            .lock()
            .map(|mut requests| std::mem::take(&mut *requests))
            .unwrap_or_default();
        self.queue_thumbnails::<Bitmaps>(kit_index, entries, 256, ctx);
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
#[path = "tests/bitmap_browser.rs"]
mod tests;

#[cfg(test)]
mod stale_result_tests {
    use super::*;

    /// A thumbnail that lands after a generation bump is dropped, but its key
    /// must leave `pending`: it used to stay, and four such keys stopped every
    /// further decode for the kit.
    #[test]
    fn a_stale_thumbnail_still_frees_its_decode_slot() {
        let mut app = Baboon::for_test();
        let stamp = app.kit_stamp();
        app.kits[0]
            .bitmap_browser
            .pending
            .insert("file:a.bitmap".to_owned());
        app.kits[0].generation = app.kits[0].generation.wrapping_add(1);

        app.handle_thumbnail_ready::<Bitmaps>(
            stamp,
            "file:a.bitmap".to_owned(),
            Err("stale".to_owned()),
            &egui::Context::default(),
        );

        assert!(app.kits[0].bitmap_browser.pending.is_empty());
        let cached = app.kits[0]
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
        let key = |path: &Path| format!("file:{}", path.display());
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
        let stamp = app.kit_stamp();
        let state = app.kits[0]
            .model_previews
            .entry("file:a.model".to_owned())
            .or_default();
        state.textures_pending = true;
        app.kits[0].generation = app.kits[0].generation.wrapping_add(1);

        app.handle_model_textures_resolved(stamp, "file:a.model".to_owned(), 1, Vec::new());

        assert!(!app.kits[0].model_previews["file:a.model"].textures_pending);
    }
}
