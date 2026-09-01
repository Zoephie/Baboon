//! The Model Library: every render model in a kit as a searchable thumbnail grid.
//! It owns the grid's state, its CPU thumbnail rasterizer, and its presentation;
//! geometry decoding, tag reading, the shared grid/cache plumbing
//! (`bitmap_browser`), and the tab layout belong elsewhere.

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

/// One kit's Model Library.
#[derive(Default)]
pub(in crate::app) struct ModelBrowserState {
    pub(in crate::app) filter: String,
    pub(in crate::app) cell_size: f32,
    /// Every render model in the kit, snapshotted rather than re-scanned per
    /// frame.
    entries: Vec<TagEntry>,
    /// The kit generation `entries` was taken at, so a reload refreshes it.
    entries_for: Option<u64>,
    /// Indices into `entries` matching `filter`.
    matches: Vec<usize>,
    /// The query `matches` was computed for.
    matched_for: Option<String>,
    thumbnails: ThumbnailCache,
    /// Keys with a rasterize job running, so a cell is not queued twice while
    /// its thread works.
    pending: HashSet<String>,
    /// Set once the "scan the whole kit" request has gone out, so the browser
    /// does not ask again every frame it is drawn.
    requested_scan: bool,
    /// A double-clicked cell waiting to open the `.model` that owns its render
    /// model (or, when no such tag exists, the render model itself).
    ///
    /// Parked for the same reason as the Bitmap Library's: the grid draws
    /// inside `tree.ui`, where the kit's `tag_tree` has been moved out, and
    /// opening there writes the tab into a placeholder that is discarded.
    /// `draw_tag_tiles` takes this once the tree is back.
    pub(in crate::app) pending_open: Option<String>,
    /// A cell whose right-click menu asked for the render model tag itself,
    /// with no owner resolution. Parked likewise.
    pub(in crate::app) pending_open_raw: Option<String>,
}

impl ModelBrowserState {
    fn cell_size(&self) -> f32 {
        if self.cell_size <= 0.0 {
            DEFAULT_CELL
        } else {
            self.cell_size.clamp(MIN_CELL, MAX_CELL)
        }
    }
}

/// What a grid cell asked for this frame. Both are parked rather than run on
/// the spot — see the fields they land in on [`ModelBrowserState`].
enum CellAction {
    Open(String),
    OpenRaw(String),
}

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

    /// Draw one kit's Model Library pane.
    pub(super) fn draw_model_library(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        self.refresh_model_library(kit_index, ctx);

        let cell = self.kits[kit_index].model_browser.cell_size();
        let total = self.kits[kit_index].model_browser.matches.len();
        let all = self.kits[kit_index].model_browser.entries.len();
        let scanning = self.kits[kit_index].scanning_entries;

        self.draw_model_library_toolbar(ui, kit_index, total, all, scanning);
        ui.separator();

        if all == 0 {
            ui.add_space(12.0);
            ui.label(
                RichText::new(if scanning {
                    "Indexing the kit — models will appear as they are found."
                } else {
                    "No render model tags in this workspace."
                })
                .color(subtle_dark()),
            );
            return;
        }
        if total == 0 {
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!(
                    "No model matches that search. {all} in this workspace."
                ))
                .color(subtle_dark()),
            );
            return;
        }

        self.draw_model_grid(ui, ctx, kit_index, cell, total);
    }

    fn draw_model_library_toolbar(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        shown: usize,
        total: usize,
        scanning: bool,
    ) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Search").color(subtle_dark()));
            let browser = &mut self.kits[kit_index].model_browser;
            ui.add(
                egui::TextEdit::singleline(&mut browser.filter)
                    .hint_text("warthog | ghost, ^objects, _lod$")
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
                format!("{total} models")
            } else {
                format!("{shown} of {total} models")
            };
            ui.label(RichText::new(count).color(subtle_dark()));
            if scanning {
                ui.spinner();
                ui.label(RichText::new("indexing…").color(subtle_dark()));
            }
        });
    }

    fn draw_model_grid(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
        cell: f32,
        total: usize,
    ) {
        let row_height = cell + CELL_CAPTION + CELL_GAP;
        // Reserve the scrollbar before dividing — see `draw_bitmap_grid`, whose
        // arithmetic (and its two off-by-a-gap traps) this shares verbatim.
        let usable = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(cell);
        let columns = grid_columns(usable, cell);
        let rows = total.div_ceil(columns);

        let mut action: Option<CellAction> = None;
        let mut wanted: Vec<String> = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt(("model_library", kit_index))
            .auto_shrink([false, false])
            .show_rows(ui, row_height, rows, |ui, row_range| {
                // The gap is the only spacing in play; egui's default
                // `item_spacing` on top of it would drift the grid out of step
                // with its own scrollbar.
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
                                self.draw_model_cell(ui, kit_index, index, cell, &mut wanted)
                            {
                                action = Some(requested);
                            }
                        }
                    });
                    ui.add_space(CELL_GAP);
                }
            });

        self.queue_model_thumbnails(kit_index, wanted, cell, ctx);
        // Parked rather than opened here, for the reason on the state fields.
        match action {
            Some(CellAction::Open(key)) => {
                self.kits[kit_index].model_browser.pending_open = Some(key)
            }
            Some(CellAction::OpenRaw(key)) => {
                self.kits[kit_index].model_browser.pending_open_raw = Some(key)
            }
            None => {}
        }
    }

    /// One grid cell, and whatever the user asked it for.
    fn draw_model_cell(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        index: usize,
        cell: f32,
        wanted: &mut Vec<String>,
    ) -> Option<CellAction> {
        let browser = &mut self.kits[kit_index].model_browser;
        let entry_index = *browser.matches.get(index)?;
        let entry = browser.entries.get(entry_index)?;
        let (key, display_path) = (entry.key.clone(), entry.display_path.clone());

        let texture = match browser.thumbnails.get(&key) {
            Some(texture) => texture,
            None => {
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
        let is_gbxmodel = group_tag == u32::from_be_bytes(*b"mod2");

        let size = Vec2::new(cell, cell + CELL_CAPTION);
        // `click_and_drag`, so a cell is both a target to open and a source to
        // drag: the payload is the browser row's own `DraggedTagRef`, which
        // reference cells already accept.
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        response.dnd_set_drag_payload(DraggedTagRef {
            group_tag,
            input: reference_input,
            rel_path,
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
            if is_gbxmodel { "Gbxmodel" } else { "Render Model" },
            FontId::proportional(10.0),
            subtle_dark(),
        );

        // The name follows the cursor while dragging, as the browser rows do.
        if response.dragged()
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            egui::Area::new(ui.make_persistent_id(("model_library_drag_preview", &key)))
                .order(egui::Order::Tooltip)
                // Never in the hit-test: a fast drag can put the pointer
                // inside the stale preview, which would block the drop target.
                .interactable(false)
                .fixed_pos(pointer + Vec2::new(12.0, 12.0))
                .show(ui.ctx(), |ui| {
                    ui.label(RichText::new(&name).color(text_dark()));
                });
        }

        // Double-click, not click, for the same reason as the Bitmap Library.
        let mut action = response
            .double_clicked()
            .then(|| CellAction::Open(key.clone()));

        response.context_menu(|ui| {
            style_tag_context_menu(ui);
            if context_menu_button(ui, "Open render model tag").clicked() {
                action = Some(CellAction::OpenRaw(key.clone()));
                ui.close_menu();
            }
        });

        if !response.context_menu_opened() {
            let opens = if is_gbxmodel {
                // Halo CE has no .model wrapper, so the double-click opens the
                // gbxmodel itself.
                "Double-click to open"
            } else {
                "Double-click to open the model that owns it"
            };
            // Not `on_hover_text`: an egui tooltip would block the drag this
            // cell offers (`hover_tooltip_beside_pointer`).
            hover_tooltip_beside_pointer(
                ui,
                &response,
                &format!(
                    "{display_path}\n\n{opens}, drag onto a model reference, \
                     or right-click to open the render model tag itself"
                ),
            );
        }
        action
    }

    /// Snapshot the kit's render models and recompute the filter, both only
    /// when something they depend on has actually changed.
    fn refresh_model_library(&mut self, kit_index: usize, ctx: &egui::Context) {
        let generation = self.kits[kit_index].generation;
        let stale = self.kits[kit_index].model_browser.entries_for != Some(generation);
        if stale {
            let entries: Vec<TagEntry> = self.kits[kit_index]
                .source
                .as_ref()
                .map(|source| source.full_entry_set())
                .unwrap_or_default()
                .iter()
                .filter(|entry| is_render_model_tag(entry))
                .cloned()
                .collect();
            let browser = &mut self.kits[kit_index].model_browser;
            browser.entries = entries;
            browser.entries_for = Some(generation);
            browser.matched_for = None;
            browser.thumbnails.clear();
            browser.requested_scan = false;
        }

        // A lazy loose kit only holds the folders the browser has expanded
        // until the full scan runs — ask for it once, as the Bitmap Library
        // does.
        let needs_scan = self.kits[kit_index]
            .source
            .as_ref()
            .is_some_and(|source| source.all_entries.is_empty())
            && !self.kits[kit_index].scanning_entries
            && !self.kits[kit_index].model_browser.requested_scan;
        if needs_scan {
            self.kits[kit_index].model_browser.requested_scan = true;
            self.begin_scan_all_entries(ctx.clone());
        }

        let browser = &mut self.kits[kit_index].model_browser;
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

    /// Start rasterize jobs for the visible cells that have none, up to the
    /// in-flight bound.
    fn queue_model_thumbnails(
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
        // Twice the cell's point size, for the slider and high-DPI displays.
        let max_edge = ((cell * 2.0).round() as u32).max(MIN_CELL as u32);

        for key in wanted {
            if self.kits[kit_index].model_browser.pending.len() >= MAX_DECODES_IN_FLIGHT {
                break;
            }
            if self.kits[kit_index].model_browser.pending.contains(&key)
                || self.kits[kit_index].model_browser.thumbnails.contains(&key)
            {
                continue;
            }
            let Some(entry) = self.kits[kit_index]
                .model_browser
                .entries
                .iter()
                .find(|entry| entry.key == key)
                .cloned()
            else {
                continue;
            };
            self.kits[kit_index]
                .model_browser
                .pending
                .insert(key.clone());

            let (tx, ctx, source) = (self.tx.clone(), ctx.clone(), source.clone());
            thread::spawn(move || {
                // `catch_unwind` because some tags panic the geometry parser:
                // a panicking thread would never send its message, `pending`
                // would never clear, and one of the four decode slots would be
                // lost for the kit's lifetime.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::source::read_entry(&source, &entry)
                        .map_err(|error| error.to_string())
                        .and_then(|tag| build_render_preview(&tag))
                        .and_then(|preview| rasterize_model_thumbnail(&preview, max_edge))
                }))
                .unwrap_or_else(|_| Err("render model crashed while parsing".to_owned()));
                let _ = tx.send(WorkerMessage::ModelThumbnailRendered { stamp, key, result });
                ctx.request_repaint();
            });
        }
    }

    pub(super) fn handle_model_thumbnail_rendered(
        &mut self,
        stamp: KitStamp,
        key: String,
        result: Result<ThumbnailImage, String>,
        ctx: &egui::Context,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            // The kit closed or was reloaded while this rendered.
            return true;
        };
        let browser = &mut self.kits[kit_index].model_browser;
        browser.pending.remove(&key);
        // A failure is cached as `None` rather than dropped, so an unparseable
        // model is not re-rasterized every frame it stays on screen.
        let texture = match result {
            Ok(image) => Some(ctx.load_texture(
                format!("model_thumb:{key}"),
                egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba),
                egui::TextureOptions::LINEAR,
            )),
            Err(_) => None,
        };
        browser.thumbnails.insert(key, texture);
        false
    }
}

#[cfg(test)]
#[path = "tests/model_browser.rs"]
mod tests;
