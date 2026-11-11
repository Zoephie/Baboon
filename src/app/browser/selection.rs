//! Ordered, pane-local selection. The same rules drive rows and asset cells.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Default, Debug)]
pub(in crate::app) struct BrowserSelection {
    pub keys: BTreeSet<String>,
    pub anchor: Option<String>,
    pub focus: Option<String>,
    pub rows: Vec<(String, String)>,
}
impl BrowserSelection {
    pub fn clear(&mut self) {
        self.keys.clear();
        self.anchor = None;
        self.focus = None;
    }
    pub fn click(&mut self, key: &str, toggle: bool, range: bool, secondary: bool) {
        if secondary && self.keys.contains(key) {
            self.focus = Some(key.into());
            return;
        }
        let (toggle, range) = if secondary {
            (false, false)
        } else {
            (toggle, range)
        };
        if range {
            if let (Some(a), Some(b)) = (
                self.anchor
                    .as_ref()
                    .and_then(|key| self.rows.iter().position(|row| &row.0 == key)),
                self.rows.iter().position(|row| row.0 == key),
            ) {
                if !toggle {
                    self.keys.clear();
                }
                self.keys.extend(
                    self.rows[a.min(b)..=a.max(b)]
                        .iter()
                        .map(|row| row.0.clone()),
                );
                self.focus = Some(key.into());
                return;
            }
        }
        if !toggle {
            self.keys.clear();
        }
        if !toggle || !self.keys.remove(key) {
            self.keys.insert(key.into());
        }
        self.anchor = Some(key.into());
        self.focus = Some(key.into());
    }
    pub fn all(&mut self) {
        self.keys = self.rows.iter().map(|row| row.0.clone()).collect();
        if self.focus.is_none() {
            self.focus = self.rows.first().map(|row| row.0.clone());
        }
    }
    pub fn retain_rows(&mut self) {
        let visible: HashSet<_> = self.rows.iter().map(|row| row.0.as_str()).collect();
        self.keys.retain(|key| visible.contains(key.as_str()));
    }
    pub fn ordered_keys(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| self.keys.contains(&row.0))
            .map(|row| row.0.clone())
            .collect()
    }
}
fn slot() -> egui::Id {
    egui::Id::unique("browser_multi_selection_draw")
}
#[derive(Clone)]
pub(super) struct MultiDraw {
    pub pane: String,
    pub kit: KitId,
    pub selection: BrowserSelection,
    pub assets: bool,
}
pub(super) fn begin_multi(
    ui: &Ui,
    pane: &str,
    kit: KitId,
    selection: &BrowserSelection,
    assets: bool,
) {
    ui.data_mut(|data| {
        data.insert_temp(
            slot(),
            Arc::new(std::sync::Mutex::new(MultiDraw {
                pane: pane.into(),
                kit,
                selection: selection.clone(),
                assets,
            })),
        )
    });
}
pub(super) fn end_multi(ui: &Ui) -> Option<MultiDraw> {
    let result = multi(ui);
    ui.data_mut(|data| data.remove::<Arc<std::sync::Mutex<MultiDraw>>>(slot()));
    result
}
pub(super) fn multi(ui: &Ui) -> Option<MultiDraw> {
    ui.data(|data| data.get_temp::<Arc<std::sync::Mutex<MultiDraw>>>(slot()))
        .map(|value| value.lock().unwrap_or_else(|e| e.into_inner()).clone())
}
pub(super) fn set_operation_entries(
    ui: &Ui,
    kit: KitId,
    generation: u64,
    entries: &[TagEntry],
    readonly: bool,
    complete: bool,
) {
    let id = egui::Id::unique(("browser_operation_entries", kit));
    let snapshot = ui.data_mut(|data| {
        let value = data.get_temp_mut_or_default::<(u64, Arc<Vec<TagEntry>>)>(id);
        if value.0 != generation || value.1.len() != entries.len() {
            *value = (generation, Arc::new(entries.to_vec()));
        }
        value.1.clone()
    });
    ui.data_mut(|data| {
        data.insert_temp(slot().with("entries"), snapshot);
        data.insert_temp(slot().with("readonly"), readonly);
        data.insert_temp(slot().with("complete"), complete);
    });
}
pub(super) fn operation_entries(ui: &Ui) -> Option<Arc<Vec<TagEntry>>> {
    ui.data(|data| data.get_temp::<Arc<Vec<TagEntry>>>(slot().with("entries")))
}
pub(super) fn extraction_catalog_complete(ui: &Ui) -> bool {
    ui.data(|data| data.get_temp::<bool>(slot().with("complete")))
        .unwrap_or(!browser_loose_source(ui))
}
pub(in crate::app) fn selection_entries<'a>(
    keys: &[String],
    entries: &'a [TagEntry],
) -> Vec<&'a TagEntry> {
    let tags: HashSet<_> = keys
        .iter()
        .filter(|key| !is_folder_pane_key(key))
        .map(String::as_str)
        .collect();
    let folders: Vec<_> = keys
        .iter()
        .filter_map(|key| key.strip_prefix(FOLDER_PANE_PREFIX))
        .map(|path| path.replace('/', "\\").to_ascii_lowercase() + "\\")
        .collect();
    entries
        .iter()
        .filter(|entry| {
            tags.contains(entry.key.as_str())
                || (!folders.is_empty() && {
                    let path = entry.display_path.to_ascii_lowercase();
                    folders.iter().any(|prefix| path.starts_with(prefix))
                })
        })
        .collect()
}
pub(in crate::app) fn export_keys(action: BulkAction, entries: &[&TagEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| match action {
            BulkAction::Bitmaps => is_bitmap_tag(entry),
            BulkAction::Sounds => crate::app::editor::is_sound_group(entry.group_tag),
            BulkAction::Shaders => is_material_shader_browser_tag(entry),
            BulkAction::Includes => is_hlsl_include_tag(entry),
            BulkAction::Raw => matches!(entry.location, TagEntryLocation::Container { .. }),
            BulkAction::Json => true,
            _ => false,
        })
        .map(|entry| entry.key.clone())
        .collect()
}
pub(super) const FLAT_ROW_CONTENT_INSET: f32 = 8.0;
pub(super) fn flat_row_content_inset(ui: &Ui) -> f32 {
    let flat = ui
        .data(|data| data.get_temp::<Arc<std::sync::Mutex<MultiDraw>>>(slot()))
        .is_some_and(|state| {
            !state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .assets
        });
    if flat { FLAT_ROW_CONTENT_INSET } else { 0.0 }
}

pub(super) fn multi_select(
    ui: &Ui,
    response: &egui::Response,
    key: &str,
    navigated: bool,
) -> Option<bool> {
    let shared = ui.data(|data| data.get_temp::<Arc<std::sync::Mutex<MultiDraw>>>(slot()))?;
    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
    if response.clicked() || response.secondary_clicked() || navigated {
        let modifiers = ui.input(|input| input.modifiers);
        state.selection.click(
            key,
            !navigated && (modifiers.command || modifiers.ctrl),
            if navigated {
                browser_navigation_extending(ui.ctx())
            } else {
                modifiers.shift
            },
            response.secondary_clicked(),
        );
        let owner = (state.kit, state.pane.clone());
        ui.data_mut(|data| data.insert_temp(browser_selection_owner_id().with("pane"), owner));
    }
    Some(state.selection.keys.contains(key))
}

/// Focus belongs to the pane, while every selected row shares its appearance.
pub(super) fn selection_is_active(ui: &Ui) -> bool {
    ui.data(|data| data.get_temp::<(egui::Id, egui::Rect)>(browser_selection_owner_id()))
        .is_some_and(|(owner, _)| {
            let same_pane = ui.data(|data| {
                let owner =
                    data.get_temp::<(KitId, String)>(browser_selection_owner_id().with("pane"));
                if browser_is_folder_pane(ui) {
                    owner
                        == data.get_temp::<(KitId, String)>(
                            browser_selection_owner_id().with("drawing"),
                        )
                } else {
                    owner.is_none()
                }
            });
            same_pane && ui.memory(|memory| memory.focused() == Some(owner))
        })
}
pub(super) fn clear_multi_on_blank(ui: &Ui, response: &egui::Response) {
    if !response.clicked() {
        return;
    }
    let Some(mut state) = multi(ui) else {
        return;
    };
    response.request_focus();
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            response.id,
            egui::EventFilter {
                vertical_arrows: true,
                horizontal_arrows: true,
                escape: true,
                ..Default::default()
            },
        )
    });
    browser_navigation_select(ui, response.id);
    ui.data_mut(|data| {
        data.insert_temp(browser_selection_owner_id(), (response.id, ui.clip_rect()));
        data.insert_temp(
            browser_selection_owner_id().with("pane"),
            (state.kit, state.pane.clone()),
        );
    });
    state.selection.clear();
    begin_multi(ui, &state.pane, state.kit, &state.selection, state.assets);
    set_browser_selected_tag(ui, None);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum BulkAction {
    Open,
    Rename,
    Move,
    Copy,
    Delete,
    Bitmaps,
    Sounds,
    Shaders,
    Includes,
    Json,
    Raw,
}

/// Menus carry stable keys, never row indices or borrowed source entries.
pub(super) fn draw_multi_menu(ui: &mut Ui) -> Option<BrowserAction> {
    let state = multi(ui)?;
    style_tag_context_menu(ui);
    let keys = state.selection.ordered_keys();
    if keys.is_empty() {
        return None;
    }
    let mut result = None;
    let snapshot = ui
        .data(|data| data.get_temp::<Arc<Vec<TagEntry>>>(slot().with("entries")))
        .unwrap_or_default();
    let entries = selection_entries(&keys, &snapshot);
    let writable = browser_loose_source(ui)
        && !ui
            .data(|data| data.get_temp::<bool>(slot().with("readonly")))
            .unwrap_or(false);
    let primary: Vec<_> = if state.assets {
        vec![
            ("Rename", BulkAction::Rename),
            ("Delete", BulkAction::Delete),
        ]
    } else {
        vec![
            ("Rename", BulkAction::Rename),
            ("Move", BulkAction::Move),
            ("Copy To", BulkAction::Copy),
            ("Delete", BulkAction::Delete),
        ]
    };
    let width = (ui.available_width() - 4.0 * (primary.len() - 1) as f32) / primary.len() as f32;
    let request = |action| BrowserAction::Bulk {
        pane: state.pane.clone(),
        keys: keys.clone(),
        action,
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (label, action) in primary {
            if context_menu_primary_button(ui, label, writable, width)
                .on_disabled_hover_text("Every selected item must support this action")
                .clicked()
            {
                result = Some(request(action));
                close_menu(ui);
            }
        }
    });
    context_menu_separator(ui);
    if context_menu_button(ui, "Open Selected Items").clicked() {
        result = Some(request(BulkAction::Open));
        close_menu(ui);
    }
    let json_count = export_keys(BulkAction::Json, &entries).len();
    if json_count > 0
        && context_menu_button(ui, "Dump Selection to JSON...")
            .on_hover_text(format!("Export {json_count} selected tags"))
            .clicked()
    {
        result = Some(request(BulkAction::Json));
        close_menu(ui);
    }
    context_menu_separator(ui);
    if let Some(action) = folder_extract_menu_from_keys(
        ui,
        &snapshot,
        "Selected Items".into(),
        PathBuf::new(),
        export_keys(BulkAction::Raw, &entries),
        export_keys(BulkAction::Bitmaps, &entries),
        export_keys(BulkAction::Sounds, &entries),
        export_keys(BulkAction::Shaders, &entries),
        export_keys(BulkAction::Includes, &entries),
        false,
        !browser_loose_source(ui),
        false,
    ) {
        result = Some(action);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selection() -> BrowserSelection {
        BrowserSelection {
            rows: ["folder", "a", "b", "c"]
                .map(|key| (key.into(), key.into()))
                .into(),
            ..Default::default()
        }
    }
    #[test]
    fn mixed_ranges_and_right_clicks_follow_display_order() {
        let mut s = selection();
        s.click("folder", false, false, false);
        s.click("b", true, false, false);
        assert_eq!(s.keys.len(), 2);
        s.click("folder", false, false, true);
        assert_eq!(s.keys.len(), 2);
        s.click("c", false, true, false);
        assert_eq!(s.ordered_keys(), vec!["b", "c"]);
        s.click("a", false, false, true);
        assert_eq!(s.ordered_keys(), vec!["a"]);
        s.all();
        assert_eq!(s.keys.len(), 4);
        s.clear();
        assert!(s.keys.is_empty());
    }
}
