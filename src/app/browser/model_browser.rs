//! The Model Library: every render model in a kit as a searchable thumbnail grid.
//! It owns what is particular to models — which tags are listed, the CPU
//! thumbnail rasterizer, and opening the `.model` that owns a render model. The
//! grid itself is `thumbnail_library`'s; geometry decoding, tag reading, and
//! the tab layout belong elsewhere.

use super::*;

/// The pane key the Model Library occupies.
///
/// Like [`BITMAP_LIBRARY_KEY`], a `tool:`-prefixed key no tag can have: the
/// pane rides the tag-tile layout while resolving to no document, so the close
/// path finds nothing dirty and the session writer skips it.
pub(in crate::app) const MODEL_LIBRARY_KEY: &str = "tool:model_library";

pub(in crate::app) const MODEL_LIBRARY_TITLE: &str = "Model Library";

/// The editor preview's default pose (see `ModelPreviewState`), so a thumbnail
/// double-clicked open looks like what was clicked.
const THUMBNAIL_YAW: f32 = -0.45;
const THUMBNAIL_PITCH: f32 = 0.25;

/// The kit entry for the `.model` (hlmt) that owns this render geometry, found
/// by the path convention `owning_model_skeleton` documents in reverse:
/// `objects/x/warthog.render_model` is owned by `objects/x/warthog.model`.
///
/// `None` for a gbxmodel — Halo CE has no hlmt wrapper, objects reference the
/// `mod2` directly — and when no sibling exists; the caller then opens the
/// clicked tag itself. Gated on the sibling's group being `hlmt` so a Halo CE
/// kit's legacy `.model` (four-CC `mode`) is never mistaken for an owner.
fn owning_model_key(entries: &[TagEntry], clicked: &TagEntry) -> Option<String> {
    if clicked.group_tag == u32::from_be_bytes(*b"mod2")
        || clicked.group_name.as_deref() == Some("gbxmodel")
    {
        return None;
    }
    let target = format!("{}.model", normalized_tag_stem(&clicked.display_path));
    let hlmt = u32::from_be_bytes(*b"hlmt");
    entries
        .iter()
        .find(|entry| {
            entry.group_tag == hlmt
                && entry.display_path.replace('\\', "/").to_ascii_lowercase() == target
        })
        .map(|entry| entry.key.clone())
}

/// A display path lowercased, forward-slashed, and stripped of its group
/// extension — the shape two tags' paths compare in.
fn normalized_tag_stem(display_path: &str) -> String {
    let normalized = display_path.replace('\\', "/").to_ascii_lowercase();
    match normalized.rsplit_once('.') {
        // Only a dot in the leaf is an extension; a dot in a folder name is not.
        Some((stem, extension)) if !extension.contains('/') && !stem.is_empty() => stem.to_owned(),
        _ => normalized,
    }
}

/// Rasterize preview geometry into a `max_edge`-square RGBA thumbnail.
///
/// A CPU rasterizer rather than the GL preview renderer: that renderer keeps a
/// single geometry resident per context, so a grid of cells would re-upload
/// every model every frame. Flat shading from the face normal with the flat
/// per-material palette, in the same default pose as the editor's preview.
pub(in crate::app) fn rasterize_model_thumbnail(
    preview: &RenderModelPreview,
    max_edge: u32,
) -> Result<ThumbnailImage, String> {
    if preview.vertices.is_empty() || preview.indices.is_empty() || preview.batches.is_empty() {
        return Err("render model has no previewable geometry".to_owned());
    }
    let (min, max) = (preview.bounds_min, preview.bounds_max);
    if !min.iter().chain(max.iter()).all(|bound| bound.is_finite()) {
        return Err("render model bounds are not finite".to_owned());
    }
    // The camera fit the GL preview uses: bounding-sphere radius, orthographic,
    // with the same 2.2 fit ratio and the same depth convention (rotated Y,
    // smaller is nearer).
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let extent = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let radius = ((extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt()
        * 0.5)
        .max(0.001);

    let edge = max_edge.clamp(8, 1024) as usize;
    let fit = edge as f32 / (radius * 2.2);
    let half = edge as f32 * 0.5;

    // Yaw about Z then pitch about X, exactly as `PreviewCamera::rotate_vector`.
    let (sy, cy) = THUMBNAIL_YAW.sin_cos();
    let (sp, cp) = THUMBNAIL_PITCH.sin_cos();
    let rotated: Vec<[f32; 3]> = preview
        .vertices
        .iter()
        .map(|vertex| {
            let x = vertex.position[0] - center[0];
            let y = vertex.position[1] - center[1];
            let z = vertex.position[2] - center[2];
            let yaw_x = x * cy - y * sy;
            let yaw_y = x * sy + y * cy;
            [yaw_x, yaw_y * cp - z * sp, yaw_y * sp + z * cp]
        })
        .collect();

    let mut depth = vec![f32::INFINITY; edge * edge];
    let mut rgba = vec![0u8; edge * edge * 4];
    // A fixed view-space light, normalized once.
    let light = {
        let length = (0.4f32 * 0.4 + 1.0 + 0.6 * 0.6).sqrt();
        [-0.4 / length, -1.0 / length, 0.6 / length]
    };
    let mut wrote = false;

    for batch in &preview.batches {
        let color = batch
            .flat_color
            .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
            .unwrap_or_else(|| material_color(batch.material_index));
        let start = batch.index_start as usize;
        let end = start
            .saturating_add(batch.index_count as usize)
            .min(preview.indices.len());
        if start >= end {
            continue;
        }
        for triangle in preview.indices[start..end].chunks_exact(3) {
            let corners = [
                rotated.get(triangle[0] as usize),
                rotated.get(triangle[1] as usize),
                rotated.get(triangle[2] as usize),
            ];
            let (Some(&pa), Some(&pb), Some(&pc)) = (corners[0], corners[1], corners[2]) else {
                continue;
            };
            if ![pa, pb, pc]
                .iter()
                .flatten()
                .all(|component| component.is_finite())
            {
                continue;
            }

            // Flat shading from the face normal, two-sided: Halo meshes are
            // frequently visible from both sides, so the z-buffer alone
            // decides occlusion and the light never blacks out a back face.
            let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            let normal = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let length =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            if length <= f32::EPSILON {
                continue;
            }
            let towards_light =
                (normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2]) / length;
            let lum = 0.35 + 0.6 * towards_light.abs();
            let shade = |channel: u8| ((channel as f32 * lum).round().min(255.0)) as u8;
            let (red, green, blue) = (shade(color.r()), shade(color.g()), shade(color.b()));

            let (x0, y0, z0) = (half + pa[0] * fit, half - pa[2] * fit, pa[1]);
            let (x1, y1, z1) = (half + pb[0] * fit, half - pb[2] * fit, pb[1]);
            let (x2, y2, z2) = (half + pc[0] * fit, half - pc[2] * fit, pc[1]);
            let area = (x1 - x0) * (y2 - y0) - (y1 - y0) * (x2 - x0);
            if area.abs() <= f32::EPSILON {
                continue;
            }
            let min_x = x0.min(x1).min(x2).floor().clamp(0.0, (edge - 1) as f32) as usize;
            let max_x = x0.max(x1).max(x2).ceil().clamp(0.0, (edge - 1) as f32) as usize;
            let min_y = y0.min(y1).min(y2).floor().clamp(0.0, (edge - 1) as f32) as usize;
            let max_y = y0.max(y1).max(y2).ceil().clamp(0.0, (edge - 1) as f32) as usize;

            for py in min_y..=max_y {
                let fy = py as f32 + 0.5;
                for px in min_x..=max_x {
                    let fx = px as f32 + 0.5;
                    let w0 = (x2 - x1) * (fy - y1) - (y2 - y1) * (fx - x1);
                    let w1 = (x0 - x2) * (fy - y2) - (y0 - y2) * (fx - x2);
                    let w2 = (x1 - x0) * (fy - y0) - (y1 - y0) * (fx - x0);
                    let inside = if area > 0.0 {
                        w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
                    } else {
                        w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
                    };
                    if !inside {
                        continue;
                    }
                    let z = (w0 * z0 + w1 * z1 + w2 * z2) / area;
                    let index = py * edge + px;
                    if z >= depth[index] {
                        continue;
                    }
                    depth[index] = z;
                    let at = index * 4;
                    rgba[at] = red;
                    rgba[at + 1] = green;
                    rgba[at + 2] = blue;
                    rgba[at + 3] = 255;
                    wrote = true;
                }
            }
        }
    }

    if !wrote {
        return Err("render model rasterized to an empty image".to_owned());
    }
    Ok(ThumbnailImage {
        rgba,
        width: edge,
        height: edge,
    })
}

impl Baboon {
    /// Open (or focus) the Model Library in the active kit.
    pub(in crate::app) fn open_model_library(&mut self) {
        let kit = self.model.active;
        if self.model.kits[kit].source.is_none() {
            self.model.status = "Load an editing kit before browsing its models".to_owned();
            return;
        }
        self.kit_and_view(kit).open_tag_pane(MODEL_LIBRARY_KEY);
    }

}

/// The Model Library's [`ThumbnailSource`].
pub(in crate::app) struct Models;

impl ThumbnailSource for Models {
    const ID_SALT: &'static str = "model_library";
    const PLURAL: &'static str = "models";
    const SEARCH_HINT: &'static str = "warthog | ghost, ^objects, _lod$";
    const INDEXING: &'static str = "Indexing the kit — models will appear as they are found.";
    const NONE_IN_KIT: &'static str = "No render model tags in this workspace.";
    const SINGULAR: &'static str = "model";
    const TEXTURE_PREFIX: &'static str = "model_thumb";
    /// The render model tag itself, with no owner resolution.
    const MENU_ITEM: &'static str = "Open render model tag";
    const LIBRARY: Library = Library::Models;
    const CRASHED: &'static str = "render model crashed while parsing";

    fn library(view: &KitView) -> &ThumbnailLibrary<Self> {
        &view.model_browser
    }

    fn library_mut(view: &mut KitView) -> &mut ThumbnailLibrary<Self> {
        &mut view.model_browser
    }

    fn lists(entry: &TagEntry) -> bool {
        is_render_model_tag(entry)
    }

    fn caption(entry: &TagEntry) -> &'static str {
        if is_gbxmodel(entry) {
            "Gbxmodel"
        } else {
            "Render Model"
        }
    }

    fn hover_hint(entry: &TagEntry) -> &'static str {
        if is_gbxmodel(entry) {
            // Halo CE has no .model wrapper, so the double-click opens the
            // gbxmodel itself.
            "Double-click to open, drag onto a model reference, \
             or right-click to open the render model tag itself"
        } else {
            "Double-click to open the model that owns it, drag onto a model reference, \
             or right-click to open the render model tag itself"
        }
    }

    fn render(
        source: &TagSource,
        entry: &TagEntry,
        max_edge: u32,
    ) -> Result<ThumbnailImage, String> {
        crate::core::source::read_entry(source, entry)
            .map_err(|error| error.to_string())
            .and_then(|tag| build_render_preview(&tag))
            .and_then(|preview| rasterize_model_thumbnail(&preview, max_edge))
    }

    fn message(
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
    ) -> WorkerMessage {
        WorkerMessage::ModelThumbnailRendered { stamp, key, result }
    }
}

fn is_gbxmodel(entry: &TagEntry) -> bool {
    entry.group_tag == u32::from_be_bytes(*b"mod2")
}

impl Model {
    /// Resolve a double-clicked render model to the tag its cell should open:
    /// the owning `.model` when the kit has one, otherwise the tag itself.
    pub(in crate::app) fn resolve_model_browser_open(&self, kit_index: usize, key: &str) -> String {
        let Some(source) = self.kits[kit_index].source.as_ref() else {
            return key.to_owned();
        };
        let entries = source.full_entry_set();
        let Some(clicked) = entries.iter().find(|entry| entry.key == key) else {
            return key.to_owned();
        };
        owning_model_key(entries, clicked).unwrap_or_else(|| key.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::import::BLAM_KEY;
    use crate::app::kits::{KitMut, tag_tree_id};

    // The Model Library's non-drawing halves: which tags it lists, which tag a
    // double-click resolves to, and what its rasterizer draws.
    //
    // The grid, its arithmetic and its cache are `thumbnail_library`'s, shared
    // with the Bitmap Library and covered with it. What is
    // covered here is what this library adds: the render-model predicate, the
    // render_model → `.model` owner resolution, and the CPU rasterizer that must
    // never panic on the geometry a shipped tag can hold.

    fn entry(display_path: &str, group: &[u8; 4], group_name: Option<&str>) -> TagEntry {
        TagEntry {
            key: format!("file:{display_path}"),
            display_path: display_path.to_owned(),
            group_tag: u32::from_be_bytes(*group),
            group_name: group_name.map(str::to_owned),
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    /// The Model Library lists render geometry by its group as its own game
    /// names it: `render_model`, Halo CE's `gbxmodel`, and Halo CE's legacy
    /// `mode`, which that game calls `model`. The geometry-less `.model`
    /// (hlmt) — also called `model` — must not list, nor a FOURCC without its
    /// game's name, nor a name on another group's FOURCC.
    #[test]
    fn render_geometry_is_listed_by_its_group_in_its_game() {
        let render_model = entry(
            "objects/warthog.render_model",
            b"mode",
            Some("render_model"),
        );
        let gbxmodel = entry("vehicles/hog.gbxmodel", b"mod2", Some("gbxmodel"));
        let halo_ce_model = entry("weapons/rifle.model", b"mode", Some("model"));
        let the_owning_model = entry("objects/warthog.model", b"hlmt", Some("model"));
        let unnamed = entry("objects/warthog.render_model", b"mode", None);
        let misnamed = entry("objects/a", b"____", Some("render_model"));
        let a_bitmap = entry("bitmaps/e.bitmap", b"bitm", Some("bitmap"));

        assert!(is_render_model_tag(&render_model));
        assert!(is_render_model_tag(&gbxmodel));
        assert!(is_render_model_tag(&halo_ce_model));
        assert!(
            !is_render_model_tag(&the_owning_model),
            "an hlmt has no geometry of its own and must not be listed"
        );
        assert!(!is_render_model_tag(&unnamed));
        assert!(!is_render_model_tag(&misnamed));
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

    /// The same ordering the Bitmap Library pins: the grid draws while the kit's
    /// `tag_tree` is moved out, so a cell's open is a command the frame applies
    /// once the tree is back.
    #[test]
    fn opening_a_model_must_wait_until_the_tag_tree_is_back() {
        const KEY: &str = "file:objects/warthog.model";

        let mut app = Baboon::for_test();
        let kit = app.model.kits[0].id;
        let taken = std::mem::replace(
            &mut app.views[kit].tag_tree,
            egui_tiles::Tree::empty(tag_tree_id(kit)),
        );
        app.commands.send(BrowserCommand::LibraryCell {
            kit,
            library: Library::Models,
            action: CellAction::Open(KEY.to_owned()),
        });
        app.views[kit].tag_tree = taken;
        app.apply_commands(&egui::Context::default());

        assert_eq!(app.model.kits[0].open_tabs, vec![KEY.to_owned()]);
    }

    /// The session writer decides the Model Library was open by looking for its
    /// pane key in `open_tabs`, so that key has to actually land there.
    #[test]
    fn an_open_model_library_shows_up_in_the_kits_open_tabs() {
        let mut kit = Kit::empty(KitId(1), TagNameIndex::default());
        let mut view = KitView::for_test(&kit);
        KitMut::new(&mut kit, &mut view).open_tag_pane(MODEL_LIBRARY_KEY);
        assert!(
            kit.open_tabs.iter().any(|key| key == MODEL_LIBRARY_KEY),
            "the session writer looks for exactly this: {:?}",
            kit.open_tabs
        );

        KitMut::new(&mut kit, &mut view).close_tag_pane(MODEL_LIBRARY_KEY);
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
            eprintln!(
                "skipping: no .render_model tags under {}",
                tags_root.display()
            );
            return;
        }

        const CELL: u32 = 192;
        let mut rendered = 0;
        for path in &models {
            let group = u32::from_be_bytes(*b"mode");
            let Ok(tag) = crate::core::source::read_tag_at_path(path, None, None, group) else {
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
        eprintln!(
            "rasterized {rendered} of {} real render models",
            models.len()
        );
    }

    /// A library asks for its own kit's scan, not the focused kit's.
    #[test]
    fn a_library_scans_its_own_kit_not_the_focused_one() {
        let root = std::env::temp_dir().join(format!(
            "baboon-library-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = Baboon::for_test();
        let second = KitId(app.model.kits[0].id.0 + 1);
        app.push_kit(Kit::empty(second, TagNameIndex::default()));
        app.model.active = 1;
        app.install_loaded_source(LoadedSourceData {
            label: "library kit".to_owned(),
            source: TagSource::LooseFolder {
                root: root.clone(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: TagNameIndex::default(),
            game: None,
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        app.model.active = 0;

        app.refresh_thumbnail_library::<Models>(1, &egui::Context::default());

        std::fs::remove_dir_all(&root).unwrap();
        assert!(app.model.kits[1].scanning_entries, "the library's kit is scanned");
        assert!(!app.model.kits[0].scanning_entries, "the focused kit is not");
    }
}
