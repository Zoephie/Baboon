//! Chimp document panes: the tile tree, texture previews and JSON views.
//! It owns drawing an open document; property and header editing live in `property_editor` and `header_ui`.

use super::*;

struct ChimpPaneBehavior<'a> {
    app: &'a mut Baboon,
    kit_index: usize,
    close_requests: Vec<String>,
    focused: Option<String>,
    close_all: bool,
    close_all_but: Option<String>,
    extract_texture: Option<String>,
    extract_mesh: Option<(String, ChimpMeshFormat)>,
    export_level: Option<(String, ChimpLevelFormat)>,
}

impl egui_tiles::Behavior<String> for ChimpPaneBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        tile_id: egui_tiles::TileId,
        pane: &mut String,
    ) -> egui_tiles::UiResponse {
        if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect())
        {
            self.focused = Some(pane.clone());
        }
        self.app.draw_chimp_document_pane(
            ui,
            self.kit_index,
            pane,
            &format!("chimp_tile_{}", tile_id.0),
        );
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &String) -> egui::WidgetText {
        let dirty = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(pane)
            .is_some_and(|document| document.dirty);
        let label = pane.rsplit('/').next().unwrap_or(pane);
        RichText::new(if dirty {
            format!("• {label}")
        } else {
            label.to_owned()
        })
        .color(text_dark())
        .into()
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        true
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(egui_tiles::Tile::Pane(package)) = tiles.get(tile_id) {
            self.close_requests.push(package.clone());
        }
        false
    }

    fn on_tab_button(
        &mut self,
        tiles: &mut egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        let Some(egui_tiles::Tile::Pane(package)) = tiles.get(tile_id) else {
            return button_response;
        };
        let package = package.clone();
        if button_response.clicked() {
            self.focused = Some(package.clone());
        }
        if button_response.middle_clicked() {
            self.close_requests.push(package.clone());
        }
        let has_texture = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(&package)
            .is_some_and(|document| !document.texture_previews.is_empty());
        let has_mesh = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(&package)
            .is_some_and(|document| document.mesh_kind.is_some());
        context_menu(&button_response, |ui| {
            if ui.button("Close").clicked() {
                self.close_requests.push(package.clone());
                close_menu(ui);
            }
            if ui.button("Close all but this").clicked() {
                self.close_all_but = Some(package.clone());
                close_menu(ui);
            }
            if ui.button("Close all").clicked() {
                self.close_all = true;
                close_menu(ui);
            }
            if has_texture {
                ui.separator();
                chimp_texture_export_menu(ui, &package, &mut self.extract_texture);
            }
            if has_mesh {
                ui.separator();
                chimp_mesh_export_menu(ui, &package, &mut self.extract_mesh);
            }
            if chimp_looks_like_level(&package) {
                ui.separator();
                chimp_level_export_menu(ui, &package, &mut self.export_level);
            }
        });
        button_response
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        row_type()
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        tiles: &egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        let base = if state.active {
            active_tab()
        } else {
            row_type()
        };
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(package))
            if self.app.kits[self.kit_index]
                .chimp
                .documents
                .get(package)
                .is_some_and(|document| document.dirty));
        if dirty {
            chimp_tint_toward(base, Color32::from_rgb(184, 134, 11), 0.20)
        } else {
            base
        }
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> Color32 {
        text_dark()
    }
}

fn chimp_tint_toward(base: Color32, accent: Color32, amount: f32) -> Color32 {
    let mix =
        |base: u8, accent: u8| (base as f32 + (accent as f32 - base as f32) * amount).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(base.r(), accent.r()),
        mix(base.g(), accent.g()),
        mix(base.b(), accent.b()),
        base.a(),
    )
}

impl Baboon {
    pub(super) fn draw_chimp_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        let Some(mut tree) = self.kits[kit_index].chimp.document_tree.take() else {
            crate::app::shell::frame::centered_empty_state(ui, "Select a package to inspect it.");
            return;
        };
        if tree.is_empty() {
            self.kits[kit_index].chimp.document_tree = Some(tree);
            crate::app::shell::frame::centered_empty_state(ui, "Select a package to inspect it.");
            return;
        }

        let mut behavior = ChimpPaneBehavior {
            app: self,
            kit_index,
            close_requests: Vec::new(),
            focused: None,
            close_all: false,
            close_all_but: None,
            extract_texture: None,
            extract_mesh: None,
            export_level: None,
        };
        tree.ui(&mut behavior, ui);
        let close_requests = std::mem::take(&mut behavior.close_requests);
        let focused = behavior.focused.take();
        let close_all = behavior.close_all;
        let close_all_but = behavior.close_all_but.take();
        let extract_texture = behavior.extract_texture.take();
        let extract_mesh = behavior.extract_mesh.take();
        let export_level = behavior.export_level.take();
        self.kits[kit_index].chimp.document_tree = Some(tree);
        self.kits[kit_index].chimp.sync_open_packages();
        if let Some(package) = focused {
            self.kits[kit_index].chimp.selected_package = Some(package);
        }

        let mut requested = if close_all {
            self.kits[kit_index].chimp.open_packages.clone()
        } else if let Some(keep) = close_all_but {
            self.kits[kit_index]
                .chimp
                .open_packages
                .iter()
                .filter(|package| *package != &keep)
                .cloned()
                .collect()
        } else {
            close_requests
        };
        requested.sort();
        requested.dedup();
        let mut blocked = false;
        for package in requested {
            if !self.close_chimp_package(kit_index, &package) {
                blocked = true;
            }
        }
        if blocked {
            self.status = "Save or discard modified Chimp packages before closing them.".to_owned();
        }
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture(kit_index, &package);
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
        }
        if let Some((package, format)) = export_level {
            self.begin_export_chimp_level(kit_index, &package, format);
        }
    }

    fn draw_chimp_document_pane(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        package: &str,
        scope: &str,
    ) {
        if !self.kits[kit_index].chimp.documents.contains_key(package) {
            ui.label("This package is no longer loaded.");
            return;
        }
        let package = package.to_owned();

        let mut save_mod = false;
        let mut extract_package = false;
        let mut extract_json = false;
        let mut extract_export = false;
        {
            let writing = self.chimp.chimp_writes.contains_key(&self.kits[kit_index].id);
            let document = self.kits[kit_index]
                .chimp
                .documents
                .get_mut(&package)
                .expect("checked above");
            ui.horizontal(|ui| {
                ui.heading(&document.package);
                ui.separator();
                save_mod = ui
                    .add_enabled(
                        document.dirty && !writing,
                        egui::Button::new("Save Chimp changes…"),
                    )
                    .on_hover_text("Save every modified Chimp package in one operation")
                    .clicked();
                extract_package = ui.button("Extract package…").clicked();
                extract_json = ui.button("Export JSON…").clicked();
                extract_export = ui.button("Extract selected export…").clicked();
            });
        }
        if save_mod {
            self.open_chimp_save_dialog(kit_index);
        }
        if extract_package {
            self.extract_chimp_package(kit_index, &package);
        }
        if extract_json {
            self.extract_chimp_json(kit_index, &package);
        }
        if extract_export {
            self.extract_chimp_export(kit_index, &package);
        }

        // Read before the document borrow: `document` borrows this kit, and the
        // preference lives on the application.
        let expert = self.prefs.expert_mode;
        let mut scan_referrers = false;
        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let document = self.kits[kit_index]
            .chimp
            .documents
            .get_mut(&package)
            .expect("checked above");
        let container = chimp_document_container_label(document, world.containers());
        ui.label(
            RichText::new(format!(
                "{} exports • {} imports • {} bytes • {}",
                document.header.export_map.len(),
                document.header.import_map.len(),
                document.original.len(),
                container
            ))
            .color(subtle_dark()),
        );
        if document.orphaned {
            ui.label(
                RichText::new(
                    "No mounted container provides this package any more. It can still be read and \
                     extracted, but not written back.",
                )
                .color(Color32::from_rgb(170, 130, 60)),
            );
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut document.view, ChimpDocumentView::Document, "Document")
                .on_hover_text("Readable JSON representation of the complete decoded package");
            if !document.texture_previews.is_empty() {
                ui.selectable_value(&mut document.view, ChimpDocumentView::Texture, "Texture")
                    .on_hover_text("Decoded Texture2D image preview");
            }
            if document.mesh_kind.is_some() {
                ui.selectable_value(&mut document.view, ChimpDocumentView::Mesh, "Mesh")
                    .on_hover_text("Decoded Unreal mesh in Baboon's 3D viewer");
            }
            ui.selectable_value(
                &mut document.view,
                ChimpDocumentView::Properties,
                "Properties",
            )
            .on_hover_text("Inspect exports and edit supported reflected scalar properties");
            ui.selectable_value(&mut document.view, ChimpDocumentView::Header, "Header")
                .on_hover_text("The package's name map, imports and exports, and what uses each");
            ui.selectable_value(&mut document.view, ChimpDocumentView::Metadata, "Metadata")
                .on_hover_text("Package dependencies and physical archive providers");
        });
        ui.separator();

        let changed = match document.view {
            ChimpDocumentView::Document => {
                if document.document_text_dirty {
                    refresh_chimp_document_text(document);
                }
                draw_chimp_json_document(
                    ui,
                    ("chimp_document_text", scope.to_owned(), package.clone()),
                    "Decoded Unreal package document",
                    "Copy JSON",
                    &document.document_text,
                    &mut document.document_lines,
                );
                false
            }
            ChimpDocumentView::Texture => {
                draw_chimp_texture_preview(ui, document, &mut self.prefs.bitmap_preview_view);
                false
            }
            ChimpDocumentView::Mesh => {
                match document.mesh_preview.as_ref() {
                    Some(Ok(preview)) => model_preview::draw_standalone_mesh_preview(
                        ui,
                        preview,
                        &mut document.mesh_preview_state,
                    ),
                    Some(Err(error)) => {
                        ui.colored_label(Color32::from_rgb(150, 56, 44), error);
                    }
                    None => {
                        ui.label(RichText::new("No mesh geometry found.").color(subtle_dark()));
                    }
                }
                false
            }
            ChimpDocumentView::Properties => {
                egui::Panel::left(egui::Id::new((
                    "chimp_exports",
                    scope.to_owned(),
                    package.clone(),
                )))
                .resizable(true)
                .default_size(220.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Exports").strong());
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (index, export) in document.exports.iter().enumerate() {
                            let supported = export.decoded.is_ok();
                            let label =
                                format!("{}  {}", if supported { "●" } else { "○" }, export.object);
                            if ui
                                .selectable_label(document.selected_export == index, label)
                                .on_hover_text(export.class.as_deref().unwrap_or("Unknown class"))
                                .clicked()
                            {
                                document.selected_export = index;
                            }
                        }
                    });
                });
                egui::CentralPanel::default()
                    .show(ui, |ui| {
                        draw_chimp_export_editor(ui, document, world.usmap())
                    })
                    .inner
            }
            ChimpDocumentView::Header => {
                draw_chimp_header_view(ui, document, &world, expert, &mut scan_referrers)
            }
            ChimpDocumentView::Metadata => {
                if document.metadata_text_dirty {
                    refresh_chimp_metadata_text(document, &world);
                }
                draw_chimp_json_document(
                    ui,
                    ("chimp_metadata_text", scope.to_owned(), package.clone()),
                    "Decoded package metadata",
                    "Copy metadata JSON",
                    &document.metadata_text,
                    &mut document.metadata_lines,
                );
                false
            }
        };
        if changed {
            document.dirty = true;
            document.edits += 1;
            document.document_text_dirty = true;
            document.metadata_text_dirty = true;
            // Reference counts are derived from the same header the metadata
            // text is, so they go stale at exactly the same moment.
            document.header_usage = None;
            document.checkpoint_due = Some(ui.input(|input| input.time) + CHIMP_CHECKPOINT_DELAY);
        }
        if scan_referrers {
            let ctx = ui.ctx().clone();
            self.begin_chimp_referrer_scan(kit_index, package.clone(), ctx);
        }
    }
}

/// Where a document's package is served from, for its header line.
///
/// An orphaned document keeps the container index it had before a remount,
/// and that index now addresses a different list: it can name the wrong
/// container, or run past the end if the list shrank, which indexing with it
/// directly turned into a panic every frame the pane was drawn.
fn chimp_document_container_label(
    document: &ChimpDocument,
    containers: &[blam_tags::iostore::world::WorldContainer],
) -> String {
    if document.orphaned {
        return "(no container)".to_owned();
    }
    containers
        .get(document.provider.container)
        .map(|container| container.path.display().to_string())
        .unwrap_or_else(|| "(no container)".to_owned())
}

fn draw_chimp_texture_preview(
    ui: &mut Ui,
    document: &mut ChimpDocument,
    view_settings: &mut BitmapPreviewViewSettings,
) {
    let options: Vec<(usize, String)> = document
        .texture_previews
        .iter()
        .map(|preview| {
            (
                preview.export_index,
                document
                    .exports
                    .get(preview.export_index)
                    .map(|export| export.object.clone())
                    .unwrap_or_else(|| format!("Export {}", preview.export_index)),
            )
        })
        .collect();
    let mut selected_export = if options
        .iter()
        .any(|(index, _)| *index == document.selected_export)
    {
        document.selected_export
    } else {
        options.first().map(|(index, _)| *index).unwrap_or(0)
    };
    if options.len() > 1 {
        let selected_label = options
            .iter()
            .find(|(index, _)| *index == selected_export)
            .map(|(_, label)| label.as_str())
            .unwrap_or("Texture export");
        egui::ComboBox::from_id_salt(("chimp_texture_export", document.package.clone()))
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for (index, label) in &options {
                    ui.selectable_value(&mut selected_export, *index, label);
                }
            });
    }
    document.selected_export = selected_export;
    let Some(preview) = document
        .texture_previews
        .iter_mut()
        .find(|preview| preview.export_index == selected_export)
    else {
        ui.label("This package has no Texture2D export to preview.");
        return;
    };
    let texture_key = format!(
        "chimp_texture_{}_{}",
        document.package, preview.export_index
    );
    let ctx = ui.ctx().clone();

    // The shared viewer clears `decoded` when the layer or mip stepper moves;
    // refill it from the surfaces already in hand rather than re-reading the
    // package. This mirrors `draw_bitmap_preview` for classic bitmap tags.
    if preview.preview.decoded.is_none() {
        preview.preview.decoded = Some(match &preview.surfaces {
            Ok(surfaces) => chimp_texture_mip_data(
                surfaces,
                preview.preview.image_index,
                preview.preview.mip_index,
            ),
            Err(error) => Err(error.clone()),
        });
        preview.preview.texture_dirty = true;
    }

    if let Ok(surfaces) = &preview.surfaces {
        let max_side = ctx.input(|input| input.max_texture_side);
        let smallest = first_displayable_mip(surfaces, max_side);
        if preview.preview.mip_index < smallest {
            // Only reachable if the driver reports a smaller limit than the
            // conservative one used at decode time.
            preview.preview.mip_index = smallest;
            preview.preview.decoded = None;
        }
        if let Some(layer) = surfaces.layers.first()
            && smallest > 0
            && let Some(base) = layer.mips.first()
        {
            ui.label(
                RichText::new(format!(
                    "Mip 0 is {}x{}, larger than this display can upload ({max_side} px); showing mip {smallest} and smaller. Export writes every mip at full size.",
                    base.width, base.height
                ))
                .color(subtle_dark()),
            );
        }
    }

    preview.preview.apply_view_settings(*view_settings);
    draw_bitmap_preview_data(ui, &ctx, &texture_key, &mut preview.preview, true, "Layer");
    *view_settings = preview.preview.view_settings();
}

#[derive(Clone, Copy)]
struct ChimpJsonPalette {
    plain: Color32,
    key: Color32,
    string: Color32,
    path: Color32,
    number: Color32,
    literal: Color32,
    punctuation: Color32,
}

impl ChimpJsonPalette {
    fn for_dark_mode(dark_mode: bool) -> Self {
        if dark_mode {
            Self {
                plain: Color32::from_rgb(210, 214, 222),
                key: Color32::from_rgb(244, 191, 92),
                string: Color32::from_rgb(190, 235, 125),
                path: Color32::from_rgb(232, 157, 222),
                number: Color32::from_rgb(242, 139, 130),
                literal: Color32::from_rgb(105, 190, 255),
                punctuation: Color32::from_rgb(105, 210, 225),
            }
        } else {
            Self {
                plain: Color32::from_rgb(45, 50, 58),
                key: Color32::from_rgb(145, 91, 0),
                string: Color32::from_rgb(55, 112, 15),
                path: Color32::from_rgb(154, 52, 136),
                number: Color32::from_rgb(185, 62, 48),
                literal: Color32::from_rgb(0, 104, 178),
                punctuation: Color32::from_rgb(0, 112, 128),
            }
        }
    }
}

fn chimp_json_layout_job(
    text: &str,
    font_id: egui::FontId,
    dark_mode: bool,
) -> egui::text::LayoutJob {
    let palette = ChimpJsonPalette::for_dark_mode(dark_mode);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let bytes = text.as_bytes();
    let mut offset = 0;
    let mut active_key = String::new();

    while offset < bytes.len() {
        let start = offset;
        let (end, color) = match bytes[offset] {
            b'"' => {
                offset += 1;
                while offset < bytes.len() {
                    match bytes[offset] {
                        b'\\' => offset = (offset + 2).min(bytes.len()),
                        b'"' => {
                            offset += 1;
                            break;
                        }
                        _ => offset += 1,
                    }
                }
                let mut after = offset;
                while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                    after += 1;
                }
                if after < bytes.len() && bytes[after] == b':' {
                    if offset >= start + 2 {
                        active_key = text[start + 1..offset - 1].to_owned();
                    }
                    (offset, palette.key)
                } else {
                    let path_value = active_key.to_ascii_lowercase().contains("path")
                        || active_key.to_ascii_lowercase().contains("package");
                    (
                        offset,
                        if path_value {
                            palette.path
                        } else {
                            palette.string
                        },
                    )
                }
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                offset += 1;
                (offset, palette.punctuation)
            }
            b'-' | b'0'..=b'9' => {
                offset += 1;
                while offset < bytes.len()
                    && matches!(
                        bytes[offset],
                        b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
                    )
                {
                    offset += 1;
                }
                (offset, palette.number)
            }
            _ if text[start..].starts_with("true") => {
                offset += 4;
                (offset, palette.literal)
            }
            _ if text[start..].starts_with("false") => {
                offset += 5;
                (offset, palette.literal)
            }
            _ if text[start..].starts_with("null") => {
                offset += 4;
                (offset, palette.literal)
            }
            byte if byte.is_ascii_whitespace() => {
                offset += 1;
                while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
                    offset += 1;
                }
                (offset, palette.plain)
            }
            _ => {
                offset += text[start..].chars().next().map_or(1, char::len_utf8);
                (offset, palette.plain)
            }
        };
        job.append(
            &text[start..end],
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    }
    job
}

/// A JSON pane's text, highlighted and split into one layout job per line.
///
/// Built the first time the pane is drawn and again only when the theme or
/// font changes; replacing the text resets it. The pane draws just the lines
/// in view from it. It used to hand egui the whole document as one label every
/// frame, which for a 100,000-line package was 18 ms a frame in a release
/// build, spent hashing, copying and laying out text nobody could see.
#[derive(Default)]
pub(super) struct ChimpJsonLines {
    key: Option<(bool, egui::FontId)>,
    lines: Vec<egui::text::LayoutJob>,
}

impl ChimpJsonLines {
    pub(super) fn lines(
        &mut self,
        text: &str,
        font_id: &egui::FontId,
        dark_mode: bool,
    ) -> &[egui::text::LayoutJob] {
        let key = (dark_mode, font_id.clone());
        if self.key.as_ref() != Some(&key) {
            self.lines =
                split_layout_job_lines(&chimp_json_layout_job(text, font_id.clone(), dark_mode));
            self.key = Some(key);
        }
        &self.lines
    }
}

/// Split a single-block layout job at its newlines, keeping every section's
/// format. The highlighter works over the whole text because a key's colour
/// carries onto the values on the lines after it; splitting after it keeps
/// that. Counts lines as `str::lines` does, with at least one.
fn split_layout_job_lines(job: &egui::text::LayoutJob) -> Vec<egui::text::LayoutJob> {
    let new_line = || {
        let mut line = egui::text::LayoutJob::default();
        line.wrap.max_width = f32::INFINITY;
        line
    };
    let mut lines = Vec::new();
    let mut current = new_line();
    for section in &job.sections {
        let mut text = &job.text[section.byte_range.start.0..section.byte_range.end.0];
        loop {
            let (piece, rest) = match text.find('\n') {
                Some(end) => (&text[..end], Some(&text[end + 1..])),
                None => (text, None),
            };
            if !piece.is_empty() {
                current.append(piece, 0.0, section.format.clone());
            }
            let Some(rest) = rest else {
                break;
            };
            lines.push(std::mem::replace(&mut current, new_line()));
            text = rest;
        }
    }
    if !current.text.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn draw_chimp_json_document(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    copy_label: &str,
    text: &str,
    lines: &mut ChimpJsonLines,
) {
    let font_id = TextStyle::Monospace.resolve(ui.style());
    let lines = lines.lines(text, &font_id, ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(subtle_dark()));
        if ui.small_button(copy_label).clicked() {
            ui.copy_text(text.to_owned());
        }
        ui.label(
            RichText::new(format!("{} lines", lines.len()))
                .small()
                .color(subtle_dark()),
        );
    });
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
    let digits = lines.len().to_string().len();
    let gutter_width =
        ui.fonts_mut(|fonts| fonts.glyph_width(&font_id, '0')) * digits as f32 + 14.0;
    let gutter_fill = ui.visuals().faint_bg_color;
    let number_color = ui.visuals().weak_text_color();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        egui::ScrollArea::both()
            .id_salt(id)
            .auto_shrink([false, false])
            .show_rows(ui, row_height, lines.len(), |ui, rows| {
                for index in rows {
                    ui.horizontal(|ui| {
                        let (gutter, _) = ui.allocate_exact_size(
                            egui::vec2(gutter_width, row_height),
                            Sense::hover(),
                        );
                        ui.painter().rect_filled(gutter, 0.0, gutter_fill);
                        ui.painter().text(
                            gutter.right_center() - egui::vec2(8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            index + 1,
                            font_id.clone(),
                            number_color,
                        );
                        ui.add_space(8.0);
                        ui.add(
                            egui::Label::new(lines[index].clone())
                                .selectable(true)
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
                }
            });
    });
}

#[cfg(test)]
mod tests;
