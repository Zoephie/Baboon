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
    pub(super) fn open_model_library(&mut self) {
        let kit = self.active;
        if self.kits[kit].source.is_none() {
            self.status = "Load an editing kit before browsing its models".to_owned();
            return;
        }
        self.kits[kit].open_tag_pane(MODEL_LIBRARY_KEY);
    }

    /// Resolve a double-clicked render model to the tag its cell should open:
    /// the owning `.model` when the kit has one, otherwise the tag itself.
    pub(super) fn resolve_model_browser_open(&self, kit_index: usize, key: &str) -> String {
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
    const CRASHED: &'static str = "render model crashed while parsing";

    fn library(kit: &Kit) -> &ThumbnailLibrary<Self> {
        &kit.model_browser
    }

    fn library_mut(kit: &mut Kit) -> &mut ThumbnailLibrary<Self> {
        &mut kit.model_browser
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
        crate::source::read_entry(source, entry)
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

#[cfg(test)]
#[path = "tests/model_browser.rs"]
mod tests;

#[cfg(test)]
mod library_scan_tests {
    use super::*;

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
        let second = KitId(app.kits[0].id.0 + 1);
        app.kits.push(Kit::empty(second, TagNameIndex::default()));
        app.active = 1;
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
        });
        app.active = 0;

        app.refresh_thumbnail_library::<Models>(1, &egui::Context::default());

        std::fs::remove_dir_all(&root).unwrap();
        assert!(app.kits[1].scanning_entries, "the library's kit is scanned");
        assert!(!app.kits[0].scanning_entries, "the focused kit is not");
    }
}
