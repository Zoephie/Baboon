//! The Model Library's non-drawing halves: which tags it lists, which tag a
//! double-click resolves to, and what its rasterizer draws.
//!
//! The grid itself needs a GPU context to say anything useful, and its
//! arithmetic and cache are the Bitmap Library's — covered there. What is
//! covered here is what this library adds: the render-model predicate, the
//! render_model → `.model` owner resolution, and the CPU rasterizer that must
//! never panic on the geometry a shipped tag can hold.

use super::*;

fn entry(display_path: &str, group: &[u8; 4], group_name: Option<&str>) -> TagEntry {
    TagEntry {
        key: format!("file:{display_path}"),
        display_path: display_path.to_owned(),
        group_tag: u32::from_be_bytes(*group),
        group_name: group_name.map(str::to_owned),
        location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
    }
}

/// `is_render_model_tag` is lenient the way `is_bitmap_tag` is — group naming
/// varies across the games — so every identification route must list, and the
/// geometry-less `.model` (hlmt) must not.
#[test]
fn every_way_a_render_model_identifies_itself_is_listed() {
    let by_fourcc = entry("objects/warthog.render_model", b"mode", None);
    let by_gbx_fourcc = entry("vehicles/hog.gbxmodel", b"mod2", None);
    let by_group_name = entry("objects/a", b"____", Some("render_model"));
    let by_gbx_group_name = entry("vehicles/b", b"____", Some("gbxmodel"));
    let by_extension = entry("objects/c.render_model", b"____", None);
    let by_gbx_extension = entry("vehicles/d.gbxmodel", b"____", None);
    let the_owning_model = entry("objects/warthog.model", b"hlmt", Some("model"));
    let a_bitmap = entry("bitmaps/e.bitmap", b"bitm", Some("bitmap"));

    assert!(is_render_model_tag(&by_fourcc));
    assert!(is_render_model_tag(&by_gbx_fourcc));
    assert!(is_render_model_tag(&by_group_name));
    assert!(is_render_model_tag(&by_gbx_group_name));
    assert!(is_render_model_tag(&by_extension));
    assert!(is_render_model_tag(&by_gbx_extension));
    assert!(
        !is_render_model_tag(&the_owning_model),
        "an hlmt has no geometry of its own and must not be listed"
    );
    assert!(!is_render_model_tag(&a_bitmap));
}

/// The pane key must be one no tag can produce, and must not collide with the
/// other synthetic panes sharing the `tool:` namespace.
#[test]
fn the_library_pane_key_cannot_collide_with_a_tag_key() {
    assert!(MODEL_LIBRARY_KEY.starts_with("tool:"));
    for prefix in ["file:", "cache:", "ublock:"] {
        assert!(!MODEL_LIBRARY_KEY.starts_with(prefix));
    }
    assert_ne!(MODEL_LIBRARY_KEY, BITMAP_LIBRARY_KEY);
    assert_ne!(MODEL_LIBRARY_KEY, BLAM_KEY);
}

/// Double-clicking a render model opens the `.model` that owns it, found by
/// swapping the extension — the same convention `owning_model_skeleton`
/// measured at 2,479 of 2,486 collision/physics tags on a real H3 kit.
#[test]
fn double_clicking_resolves_the_owning_model_tag() {
    let render = entry(
        "objects\\vehicles\\warthog\\warthog.render_model",
        b"mode",
        Some("render_model"),
    );
    // Different separators and case, as two build steps can leave them.
    let model = entry(
        "Objects/Vehicles/Warthog/Warthog.model",
        b"hlmt",
        Some("model"),
    );
    let entries = vec![render.clone(), model.clone()];

    assert_eq!(owning_model_key(&entries, &render), Some(model.key));
}

/// A render model with no `.model` beside it opens itself — an owner that does
/// not exist must resolve to `None`, not to a guessed key.
#[test]
fn a_render_model_with_no_model_beside_it_opens_itself() {
    let render = entry("objects/orphan.render_model", b"mode", Some("render_model"));
    let unrelated = entry("objects/other.model", b"hlmt", Some("model"));
    let entries = vec![render.clone(), unrelated];

    assert_eq!(owning_model_key(&entries, &render), None);
}

/// Halo CE has no hlmt wrapper: objects reference the gbxmodel directly, and
/// its legacy `.model` group is four-CC `mode` — a sibling that must never be
/// mistaken for an owner.
#[test]
fn a_gbxmodel_never_redirects_to_a_legacy_dot_model() {
    let gbx = entry("vehicles\\hog\\hog.gbxmodel", b"mod2", Some("gbxmodel"));
    let legacy = entry("vehicles\\hog\\hog.model", b"mode", Some("model"));
    let entries = vec![gbx.clone(), legacy.clone()];

    assert_eq!(owning_model_key(&entries, &gbx), None);
    // And the hlmt gate holds for a `mode` render model too: an H1 kit's
    // legacy `.model` is not an owner either.
    let render = entry("vehicles\\hog\\hog.render_model", b"mode", None);
    let entries = vec![render.clone(), legacy];
    assert_eq!(owning_model_key(&entries, &render), None);
}

/// The same parking-order bug the Bitmap Library pins: the grid draws while
/// the kit's `tag_tree` is moved out, so opening during the walk writes into a
/// placeholder that is thrown away.
#[test]
fn opening_a_model_must_wait_until_the_tag_tree_is_back() {
    const KEY: &str = "file:objects/warthog.model";

    let mut kit = Kit::empty(KitId(1), TagNameIndex::default());
    let taken = std::mem::replace(
        &mut kit.tag_tree,
        egui_tiles::Tree::empty(tag_tree_id(kit.id)),
    );
    kit.model_browser.pending_open = Some(KEY.to_owned());
    kit.tag_tree = taken;
    if let Some(key) = kit.model_browser.pending_open.take() {
        kit.open_tag_pane(&key);
    }

    assert_eq!(kit.open_tabs, vec![KEY.to_owned()]);
    assert!(
        kit.model_browser.pending_open.is_none(),
        "the request is one-shot; leaving it set reopens the tab every frame"
    );
}

/// The session writer decides the Model Library was open by looking for its
/// pane key in `open_tabs`, so that key has to actually land there.
#[test]
fn an_open_model_library_shows_up_in_the_kits_open_tabs() {
    let mut kit = Kit::empty(KitId(1), TagNameIndex::default());
    kit.open_tag_pane(MODEL_LIBRARY_KEY);
    assert!(
        kit.open_tabs.iter().any(|key| key == MODEL_LIBRARY_KEY),
        "the session writer looks for exactly this: {:?}",
        kit.open_tabs
    );

    kit.close_tag_pane(MODEL_LIBRARY_KEY);
    assert!(!kit.open_tabs.iter().any(|key| key == MODEL_LIBRARY_KEY));
}

fn vertex(position: [f32; 3]) -> RenderModelPreviewVertex {
    RenderModelPreviewVertex {
        position,
        ..Default::default()
    }
}

fn triangle_preview() -> RenderModelPreview {
    RenderModelPreview {
        vertices: vec![
            vertex([0.0, 0.0, 0.0]),
            vertex([1.0, 0.0, 0.0]),
            vertex([0.0, 0.0, 1.0]),
        ],
        indices: vec![0, 1, 2],
        batches: vec![RenderModelPreviewBatch {
            material_index: 0,
            index_start: 0,
            index_count: 3,
            ..Default::default()
        }],
        bounds_min: [0.0, 0.0, 0.0],
        bounds_max: [1.0, 0.0, 1.0],
        ..Default::default()
    }
}

/// One real triangle must land pixels — opaque where it covers, transparent
/// where it does not, so the cell's own background shows around the model.
#[test]
fn a_triangle_rasterizes_to_pixels_inside_a_transparent_frame() {
    const EDGE: usize = 64;
    let image = rasterize_model_thumbnail(&triangle_preview(), EDGE as u32)
        .expect("a plain triangle must rasterize");

    assert_eq!((image.width, image.height), (EDGE, EDGE));
    assert_eq!(image.rgba.len(), EDGE * EDGE * 4);
    let opaque = image.rgba.chunks_exact(4).filter(|px| px[3] == 255).count();
    let transparent = image.rgba.chunks_exact(4).filter(|px| px[3] == 0).count();
    assert!(opaque > 0, "the triangle covered no pixel at all");
    assert!(
        transparent > 0,
        "a single triangle cannot cover the whole square cell"
    );
    assert_eq!(opaque + transparent, EDGE * EDGE, "no half-written pixels");
}

/// An empty preview is an error string, not a blank texture the cache would
/// keep as if it had succeeded.
#[test]
fn a_preview_with_no_geometry_is_an_error_not_a_blank_image() {
    assert!(rasterize_model_thumbnail(&RenderModelPreview::default(), 64).is_err());
}

/// Every vertex on one point: the radius clamps, every triangle is zero-area,
/// and the answer is an error — never a panic or a divide by zero.
#[test]
fn a_degenerate_model_errors_instead_of_panicking() {
    let mut preview = triangle_preview();
    for vertex in &mut preview.vertices {
        vertex.position = [2.0, 2.0, 2.0];
    }
    preview.bounds_min = [2.0, 2.0, 2.0];
    preview.bounds_max = [2.0, 2.0, 2.0];

    assert!(rasterize_model_thumbnail(&preview, 64).is_err());
}

/// A NaN vertex skips its triangle rather than smearing NaN through the depth
/// buffer, and out-of-range indices are skipped rather than read.
#[test]
fn broken_geometry_is_skipped_triangle_by_triangle() {
    let mut preview = triangle_preview();
    preview.vertices[0].position = [f32::NAN, 0.0, 0.0];
    assert!(
        rasterize_model_thumbnail(&preview, 64).is_err(),
        "its only triangle was skipped, so the image is empty"
    );

    let mut preview = triangle_preview();
    preview.indices = vec![0, 1, 9]; // off the end of the vertex list
    assert!(rasterize_model_thumbnail(&preview, 64).is_err());

    let mut preview = triangle_preview();
    preview.batches[0].index_count = 300; // past the end of the index list
    assert!(
        rasterize_model_thumbnail(&preview, 64).is_ok(),
        "the range is clamped to the indices that exist, which still draw"
    );
}

/// Point this at an editing kit's `tags` folder to exercise the pipeline
/// against real render models. Absent, this self-skips like the rest.
const KIT_TAGS_ENV: &str = "BABOON_MODEL_KIT";

/// Parse and rasterize real render models out of a real kit at thumbnail size.
/// The unit tests above pin the arithmetic; this pins it against shipped
/// geometry, which is where degenerate normals and odd batches actually live.
#[test]
fn real_kit_render_models_rasterize_at_thumbnail_size() {
    let Some(tags_root) = std::env::var_os(KIT_TAGS_ENV).map(PathBuf::from) else {
        eprintln!("skipping: set {KIT_TAGS_ENV} to an editing kit's tags folder");
        return;
    };
    if !tags_root.is_dir() {
        eprintln!("skipping: {} is not a folder", tags_root.display());
        return;
    }

    let models: Vec<PathBuf> = walkdir::WalkDir::new(&tags_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|found| {
            found.file_type().is_file()
                && found
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("render_model"))
        })
        .map(|found| found.path().to_path_buf())
        .take(25)
        .collect();
    if models.is_empty() {
        eprintln!("skipping: no .render_model tags under {}", tags_root.display());
        return;
    }

    const CELL: u32 = 192;
    let mut rendered = 0;
    for path in &models {
        let group = u32::from_be_bytes(*b"mode");
        let Ok(tag) = crate::source::read_tag_at_path(path, None, None, group) else {
            continue;
        };
        let Ok(preview) = build_render_preview(&tag) else {
            continue;
        };
        let Ok(image) = rasterize_model_thumbnail(&preview, CELL) else {
            continue;
        };
        assert_eq!(
            image.rgba.len(),
            image.width * image.height * 4,
            "{} produced a buffer that is not tightly packed RGBA8",
            path.display()
        );
        assert!(
            image.rgba.chunks_exact(4).any(|px| px[3] == 255),
            "{} rasterized to a fully transparent image",
            path.display()
        );
        rendered += 1;
    }
    assert!(
        rendered > 0,
        "{} render models found under {} and none rasterized",
        models.len(),
        tags_root.display()
    );
    eprintln!("rasterized {rendered} of {} real render models", models.len());
}
