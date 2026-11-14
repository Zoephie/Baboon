//! User-added application launchers and their Add Shortcut dialog.
use super::*;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(in crate::app) struct AppShortcut {
    #[serde(default = "new_shortcut_id")]
    pub id: String,
    #[serde(default)]
    pub kit: Option<String>,
    pub name: String,
    pub path: PathBuf,
}

fn new_shortcut_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl AppShortcut {
    pub(in crate::app) fn key(&self) -> String {
        format!("app:{}", self.id)
    }
}

fn editable_builtin(key: &str) -> bool {
    matches!(key, "builtin:tag_test" | "builtin:sapien")
}

fn builtin_override<'a>(prefs: &'a GuiPrefs, kit: &str, key: &str) -> Option<&'a AppShortcut> {
    if !editable_builtin(key) {
        return None;
    }
    prefs
        .kit_shortcut_layouts
        .get(kit)
        .and_then(|layout| layout.overrides.get(key))
        .or_else(|| prefs.shortcut_overrides.get(key))
}

fn global_shortcut(prefs: &GuiPrefs, kit: &str, key: &str) -> Option<AppShortcut> {
    let app = if editable_builtin(key) {
        builtin_override(prefs, kit, key)
    } else {
        prefs.app_shortcuts.iter().find(|app| app.key() == key)
    }?;
    app.kit.is_none().then(|| app.clone())
}

fn open_shortcut_kits(model: &Model) -> Vec<String> {
    (0..model.kits.len())
        .map(|index| shortcut_kit_key(model, index))
        .collect()
}

pub(in crate::app) fn migrate_blender_shortcut(prefs: &mut GuiPrefs) {
    if prefs.blender_shortcut_migrated {
        return;
    }
    if let Some(path) = &prefs.blender_path {
        if !prefs
            .app_shortcuts
            .iter()
            .any(|app| same_recent_path(&app.path, path))
        {
            prefs.app_shortcuts.push(AppShortcut {
                id: new_shortcut_id(),
                kit: None,
                name: app_name(path),
                path: path.clone(),
            });
        }
    }
    prefs.blender_shortcut_migrated = true;
}

#[derive(Clone, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(in crate::app) struct ShortcutLayout {
    pub order: Vec<String>,
    pub overrides: HashMap<String, AppShortcut>,
    pub hidden: HashSet<String>,
}

pub(in crate::app) fn shortcut_kit_key(model: &Model, kit: usize) -> String {
    if let Some(profile) = &model.kits[kit].profile {
        return format!("profile:{}", profile.id);
    }
    if let Some(root) = model.loaded_tags_root_for(kit).or_else(|| {
        model.kits[kit]
            .source
            .as_ref()
            .map(|source| source.source.root_path().to_owned())
    }) {
        return shortcut_path_key(root);
    }
    format!("workspace:{}", model.kits[kit].id.0)
}

fn shortcut_path_key(root: PathBuf) -> String {
    let path = clean_recent_path(root).to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let path = path.to_lowercase();
    format!("tags:{path}")
}

fn existing_shortcut_kits(prefs: &GuiPrefs, kit: &str, open_kits: &[String]) -> Vec<String> {
    let mut kits: Vec<String> = std::iter::once(kit.to_owned())
        .chain(open_kits.iter().cloned())
        .chain(
            prefs
                .custom_editing_kit_profiles
                .iter()
                .map(|profile| format!("profile:{}", profile.id)),
        )
        .chain(prefs.kit_shortcut_layouts.keys().cloned())
        .chain(prefs.app_shortcuts.iter().filter_map(|app| app.kit.clone()))
        .chain(
            prefs
                .editing_kit_favorites
                .iter()
                .map(|favorites| shortcut_path_key(favorites.tags_root.clone())),
        )
        .chain(prefs.recent_folders.iter().cloned().map(shortcut_path_key))
        .collect();
    kits.sort();
    kits.dedup();
    kits
}

pub(in crate::app) fn shortcut_keys(prefs: &GuiPrefs, kit: &str) -> Vec<String> {
    let available: Vec<String> = ["builtin:git_review", "builtin:tag_test", "builtin:sapien"]
        .into_iter()
        .map(str::to_owned)
        .chain(
            prefs
                .app_shortcuts
                .iter()
                .filter(|app| app.kit.as_deref().is_none_or(|scope| scope == kit))
                .map(AppShortcut::key),
        )
        .collect();
    let layout = prefs.kit_shortcut_layouts.get(kit);
    let order = layout
        .map(|layout| &layout.order)
        .unwrap_or(&prefs.shortcut_order);
    let hidden = layout
        .map(|layout| &layout.hidden)
        .unwrap_or(&prefs.hidden_shortcuts);
    let mut keys = Vec::new();
    for key in order.iter().chain(available.iter()) {
        if available.contains(key)
            && (key == "builtin:git_review" || !hidden.contains(key))
            && !keys.contains(key)
        {
            keys.push(key.clone());
        }
    }
    keys
}

fn local_layout<'a>(prefs: &'a mut GuiPrefs, kit: &str) -> &'a mut ShortcutLayout {
    prefs
        .kit_shortcut_layouts
        .entry(kit.to_owned())
        .or_insert_with(|| ShortcutLayout {
            order: prefs.shortcut_order.clone(),
            hidden: prefs.hidden_shortcuts.clone(),
            overrides: HashMap::new(),
        })
}

fn reorder_shortcut(prefs: &mut GuiPrefs, kit: &str, source: &str, target: &str, after: bool) {
    let mut keys = shortcut_keys(prefs, kit);
    if source == target || !keys.iter().any(|key| key == target) {
        return;
    }
    let Some(from) = keys.iter().position(|key| key == source) else {
        return;
    };
    let source = keys.remove(from);
    let target = keys.iter().position(|key| key == target).unwrap();
    keys.insert(target + usize::from(after), source);
    local_layout(prefs, kit).order = keys;
}

fn remove_shortcut(prefs: &mut GuiPrefs, kit: &str, key: &str) {
    if key == "builtin:git_review" {
        return;
    }
    if editable_builtin(key) {
        let layout = local_layout(prefs, kit);
        layout.hidden.insert(key.to_owned());
        layout.order.retain(|entry| entry != key);
        return;
    }
    // A shared definition stays available to other kits when this kit removes it.
    let shared = prefs
        .app_shortcuts
        .iter()
        .any(|app| app.key() == key && app.kit.is_none());
    if !shared {
        prefs
            .app_shortcuts
            .retain(|app| app.key() != key || app.kit.as_deref() != Some(kit));
    }
    let layout = local_layout(prefs, kit);
    layout.order.retain(|entry| entry != key);
    if shared {
        layout.hidden.insert(key.to_owned());
    }
}

pub(in crate::app) fn builtin_shortcut(
    prefs: &GuiPrefs,
    kit: &str,
    key: &str,
    root: Option<&Path>,
    tag_test: &str,
) -> AppShortcut {
    let (name, exe) = match key {
        "builtin:git_review" => ("Git Review", None),
        "builtin:tag_test" => ("Tag Test", Some(tag_test)),
        _ => ("Sapien", Some("sapien.exe")),
    };
    let path = root
        .zip(exe)
        .map(|(root, exe)| root.join(exe))
        .unwrap_or_default();
    if let Some(app) = builtin_override(prefs, kit, key) {
        let mut app = app.clone();
        if app.path.as_os_str().is_empty() {
            app.path = path;
        }
        return app;
    }
    AppShortcut {
        id: key.to_owned(),
        kit: Some(kit.to_owned()),
        name: name.to_owned(),
        path,
    }
}

fn save_shortcut(
    prefs: &mut GuiPrefs,
    editing: Option<&str>,
    kit: &str,
    global: bool,
    mut app: AppShortcut,
    open_kits: &[String],
) {
    app.kit = (!global).then(|| kit.to_owned());
    if let Some(key) = editing {
        if key == "builtin:git_review" {
            return;
        }
        if editable_builtin(key) {
            if global {
                for layout in prefs.kit_shortcut_layouts.values_mut() {
                    if layout
                        .overrides
                        .get(key)
                        .is_some_and(|local| same_recent_path(&local.path, &app.path))
                    {
                        layout.overrides.remove(key);
                    }
                    layout.hidden.remove(key);
                }
                prefs.hidden_shortcuts.remove(key);
                prefs.shortcut_overrides.insert(key.to_owned(), app);
                local_layout(prefs, kit).overrides.remove(key);
            } else {
                let was_global =
                    builtin_override(prefs, kit, key).is_some_and(|app| app.kit.is_none());
                if was_global && let Some(shared) = prefs.shortcut_overrides.remove(key) {
                    for existing in existing_shortcut_kits(prefs, kit, open_kits) {
                        let mut local = shared.clone();
                        local.kit = Some(existing.clone());
                        local_layout(prefs, &existing)
                            .overrides
                            .entry(key.to_owned())
                            .or_insert(local);
                    }
                }
                local_layout(prefs, kit)
                    .overrides
                    .insert(key.to_owned(), app);
            }
            return;
        }
        if global {
            if prefs.app_shortcuts.iter().any(|existing| existing.key() == key) {
                share_matching_shortcuts(prefs, Some(key), app);
            }
            return;
        }
        let Some(index) = prefs
            .app_shortcuts
            .iter()
            .position(|existing| existing.key() == key)
        else {
            return;
        };
        if prefs.app_shortcuts[index].kit.is_none() {
            // Retire the global definition, retaining independent copies in existing kits.
            let shared = prefs.app_shortcuts[index].clone();
            let mut copies = Vec::new();
            for existing in existing_shortcut_kits(prefs, kit, open_kits) {
                let mut order = shortcut_keys(prefs, &existing);
                if existing != kit && !order.iter().any(|entry| entry == key) {
                    continue; // A kit that removed this shortcut keeps it removed.
                }
                let mut local = if existing == kit {
                    app.clone()
                } else {
                    shared.clone()
                };
                local.kit = Some(existing.clone());
                local.id = if existing == kit {
                    shared.id.clone()
                } else {
                    new_shortcut_id()
                };
                let local_key = local.key();
                for entry in &mut order {
                    if entry == key {
                        *entry = local_key.clone();
                    }
                }
                copies.push((existing, local, order));
            }
            prefs.app_shortcuts.remove(index);
            for (existing, local, order) in copies {
                prefs.app_shortcuts.push(local);
                let layout = local_layout(prefs, &existing);
                layout.hidden.remove(key);
                layout.order = order;
            }
        } else {
            prefs.app_shortcuts[index] = app;
        }
    } else if global {
        share_matching_shortcuts(prefs, None, app);
    } else {
        prefs.app_shortcuts.push(app);
    }
}

// Rejoin copies by executable path, so different app versions stay independent.
fn share_matching_shortcuts(prefs: &mut GuiPrefs, editing: Option<&str>, mut app: AppShortcut) {
    let canonical = editing
        .and_then(|key| prefs.app_shortcuts.iter().find(|existing| existing.key() == key))
        .or_else(|| {
            prefs.app_shortcuts.iter().find(|existing| {
                existing.kit.is_none() && same_recent_path(&existing.path, &app.path)
            })
        })
        .or_else(|| prefs.app_shortcuts.iter().find(|existing| same_recent_path(&existing.path, &app.path)));
    if let Some(existing) = canonical {
        app.id = existing.id.clone();
    }
    app.kit = None;
    let key = app.key();
    let merged: HashSet<String> = prefs.app_shortcuts.iter()
        .filter(|existing| existing.key() == key || same_recent_path(&existing.path, &app.path))
        .map(AppShortcut::key)
        .collect();
    let remap_order = |order: &mut Vec<String>| {
        let mut seen = HashSet::new();
        for entry in order.iter_mut() {
            if merged.contains(entry) {
                *entry = key.clone();
            }
        }
        order.retain(|entry| seen.insert(entry.clone()));
    };
    remap_order(&mut prefs.shortcut_order);
    prefs.hidden_shortcuts.retain(|entry| entry != &key && !merged.contains(entry));
    for layout in prefs.kit_shortcut_layouts.values_mut() {
        remap_order(&mut layout.order);
        layout.hidden.retain(|entry| entry != &key && !merged.contains(entry));
    }
    if let Some(existing) = prefs.app_shortcuts.iter_mut().find(|existing| existing.key() == key) {
        *existing = app;
    } else {
        prefs.app_shortcuts.push(app);
    }
    prefs.app_shortcuts.retain(|existing| existing.key() == key || !merged.contains(&existing.key()));
}

#[derive(Clone)]
struct ShortcutDrag {
    key: String,
    area: egui::Id,
}

fn make_shortcut_local(
    prefs: &mut GuiPrefs,
    key: &str,
    kit: &str,
    app: AppShortcut,
    open_kits: &[String],
    remove_others: bool,
) {
    if remove_others {
        for other in existing_shortcut_kits(prefs, kit, open_kits) {
            if other != kit {
                remove_shortcut(prefs, &other, key);
            }
        }
    }
    save_shortcut(prefs, Some(key), kit, false, app, open_kits);
}

pub(in crate::app) fn shortcut_interactions(
    cx: &Ctx,
    ui: &mut Ui,
    response: &egui::Response,
    key: &str,
    area: egui::Id,
    root: Option<&Path>,
    kit_index: usize,
) {
    let kit = shortcut_kit_key(cx.model, kit_index);
    response.dnd_set_drag_payload(ShortcutDrag {
        key: key.to_owned(),
        area,
    });
    let after = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|pos| pos.y > response.rect.center().y);
    if let Some(drag) = response.dnd_hover_payload::<ShortcutDrag>() {
        if drag.area == area && drag.key != key {
            let y = if after {
                response.rect.bottom()
            } else {
                response.rect.top()
            };
            ui.painter().hline(
                response.rect.x_range(),
                y,
                Stroke::new(2.0, ui.visuals().selection.stroke.color),
            );
        }
    }
    if let Some(drag) = response.dnd_release_payload::<ShortcutDrag>() {
        if drag.area == area && drag.key != key {
            let source = drag.key.clone();
            let target = key.to_owned();
            let kit = kit.clone();
            cx.edit_prefs(move |prefs| reorder_shortcut(prefs, &kit, &source, &target, after));
        }
    }
    if key == "builtin:git_review" {
        return;
    }
    response.context_menu(|ui| {
        crate::app::browser::style_tag_context_menu(ui);
        if crate::app::browser::context_menu_button(ui, "Edit Shortcut...").clicked() {
            let app = if editable_builtin(key) {
                Some(builtin_shortcut(
                    &cx.model.prefs,
                    &kit,
                    key,
                    root,
                    cx.model.tag_test_executable_for(kit_index),
                ))
            } else {
                cx.model
                    .prefs
                    .app_shortcuts
                    .iter()
                    .find(|app| app.key() == key)
                    .cloned()
            };
            if let Some(app) = app {
                cx.open_dialog(AddShortcutDialog {
                    name: app.name.clone(),
                    path: app.path.display().to_string(),
                    kit: kit.clone(),
                    browse_root: cx.model.editing_kit_root_for(kit_index),
                    global: app.kit.is_none(),
                    builtin_default_path: (editable_builtin(key)
                        && builtin_override(&cx.model.prefs, &kit, key)
                            .is_none_or(|app| app.path.as_os_str().is_empty()))
                    .then(|| app.path.clone()),
                    editing: Some(key.to_owned()),
                    ..Default::default()
                });
            }
            ui.close();
        }
        if crate::app::browser::context_menu_button(ui, "Remove Shortcut").clicked() {
            if let Some(dialog) = AddShortcutDialog::for_global_removal(cx.model, kit_index, key) {
                cx.open_dialog(dialog);
            } else {
                let key = key.to_owned();
                let kit = kit.clone();
                cx.edit_prefs(move |prefs| remove_shortcut(prefs, &kit, &key));
            }
            ui.close();
        }
    });
}

#[derive(Default)]
pub(in crate::app) struct AddShortcutDialog {
    editing: Option<String>,
    kit: String,
    browse_root: Option<PathBuf>,
    global: bool,
    builtin_default_path: Option<PathBuf>,
    name: String,
    path: String,
    error: Option<String>,
    confirmation: Option<ShortcutScopePrompt>,
    remove_from_other_kits: Option<bool>,
}

#[derive(Clone, Copy)]
enum ShortcutScopePrompt {
    MakeLocal,
    Remove { from_editor: bool },
}

impl AddShortcutDialog {
    pub(in crate::app) fn for_kit(model: &Model, kit: usize) -> Self {
        Self {
            kit: shortcut_kit_key(model, kit),
            browse_root: model.editing_kit_root_for(kit),
            ..Default::default()
        }
    }

    pub(in crate::app) fn for_global_removal(model: &Model, kit: usize, key: &str) -> Option<Self> {
        let kit = shortcut_kit_key(model, kit);
        let app = global_shortcut(&model.prefs, &kit, key)?;
        Some(Self {
            editing: Some(key.to_owned()),
            kit,
            name: app.name,
            path: app.path.display().to_string(),
            global: true,
            confirmation: Some(ShortcutScopePrompt::Remove { from_editor: false }),
            ..Default::default()
        })
    }

    fn show_scope_prompt(&mut self, cx: &Ctx) -> bool {
        let prompt = self.confirmation.unwrap();
        let removing = matches!(prompt, ShortcutScopePrompt::Remove { .. });
        let title = "Remove Global Shortcut";
        let message = if removing {
            format!(
                "Remove “{}” from this Editing Kit? You can also remove it from the other Editing Kits now, or keep those copies to remove manually.",
                self.name
            )
        } else {
            format!(
                "When you save, “{}” will no longer be Global. Remove it from the other Editing Kits now, or keep those copies to remove manually?",
                self.name
            )
        };
        let mut choice = None;
        let mut cancel = false;
        let margin = egui::Margin {
            left: 20, right: 20, top: 0, bottom: 0,
        };
        egui::Modal::new(egui::Id::unique("global_shortcut_confirmation"))
            .frame(egui::Frame::window(&cx.egui.global_style()).inner_margin(margin))
            .show(cx.egui, |ui| {
                // The custom header extends through the frame's actual padding.
                ui.spacing_mut().window_margin = margin;
                ui.set_width(window_width(cx.egui, 540.0));
                crate::app::search::draw_icon_window_header_without_close(ui, title, ButtonIcon::Pin);
                ui.add_space(12.0);
                ui.label(message);
                ui.add_space(8.0);
                ui.label("Copies you keep will become local. New Editing Kit profiles won’t inherit this global setup.");
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 40.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let background = ui.max_rect().expand2(Vec2::new(20.0, 0.0));
                        let mut painter = ui.painter().clone();
                        painter.set_clip_rect(background.intersect(cx.egui.content_rect()));
                        let mut rounding = ui.visuals().window_corner_radius;
                        rounding.nw = 0;
                        rounding.ne = 0;
                        painter.rect_filled(background, rounding, foundation_block_bar());
                        cancel = ui.button("Cancel").clicked();
                        if ui.button("Keep Others as Copies").clicked() { choice = Some(false); }
                        if ui.button("Remove From Other Kits").clicked() { choice = Some(true); }
                    },
                );
            });
        if cancel {
            self.confirmation = None;
            return match prompt {
                ShortcutScopePrompt::MakeLocal => {
                    self.global = true;
                    self.remove_from_other_kits = None;
                    true
                }
                ShortcutScopePrompt::Remove { from_editor } => from_editor,
            };
        }
        let Some(remove_others) = choice else {
            return true;
        };
        self.confirmation = None;
        match prompt {
            ShortcutScopePrompt::MakeLocal => {
                self.remove_from_other_kits = Some(remove_others);
                true
            }
            ShortcutScopePrompt::Remove { .. } => {
                let Some(key) = self.editing.clone() else {
                    return false;
                };
                let kit = self.kit.clone();
                let open_kits = open_shortcut_kits(cx.model);
                cx.edit_prefs(move |prefs| {
                    if let Some(app) = global_shortcut(prefs, &kit, &key) {
                        make_shortcut_local(prefs, &key, &kit, app, &open_kits, remove_others);
                        remove_shortcut(prefs, &kit, &key);
                    }
                });
                false
            }
        }
    }

    fn select_path(&mut self, path: PathBuf) {
        self.name = app_name(&path);
        self.path = path.display().to_string();
        self.error = None;
    }

    fn shortcut(&self) -> Result<AppShortcut, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Enter an app name.".to_owned());
        }
        let path = PathBuf::from(self.path.trim().trim_matches('"'));
        let internal = self.editing.as_deref() == Some("builtin:git_review");
        if !internal {
            crate::core::app_shortcut::validate_app_path(&path)?;
        }
        Ok(AppShortcut {
            id: self
                .editing
                .as_deref()
                .and_then(|key| key.strip_prefix("app:"))
                .map(str::to_owned)
                .unwrap_or_else(new_shortcut_id),
            kit: (!self.global).then(|| self.kit.clone()),
            name: name.to_owned(),
            path,
        })
    }
}

impl Dialog for AddShortcutDialog {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        if self.confirmation.is_some() {
            return self.show_scope_prompt(cx);
        }
        let ctx = cx.egui;
        let mut cancel = false;
        let mut save = false;
        let mut remove = false;
        let editing = self.editing.is_some();
        let internal = self.editing.as_deref() == Some("builtin:git_review");
        let title = if editing {
            "Edit Shortcut"
        } else {
            "Add Shortcut"
        };
        let margin = egui::Margin {
            left: 20,
            right: 20,
            top: 0,
            bottom: 0,
        };
        let _window = egui::Window::new(title)
            .id(egui::Id::unique("add_shortcut_dialog"))
            .constrain_to(window_work_area(ctx))
            .title_bar(false)
            .collapsible(false)
            .auto_sized()
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(margin))
            .default_width(window_width(ctx, 580.0))
            .max_width(window_width(ctx, 580.0))
            .show(ctx, |ui| {
                ui.spacing_mut().window_margin = margin;
                crate::app::search::draw_icon_window_header_without_close(ui, title, ButtonIcon::Pin);
                ui.add_space(8.0);
                let banner = ui
                    .allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), 52.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let background = ui.max_rect().expand2(Vec2::new(12.0, 0.0));
                            let mut painter = ui.painter().clone();
                            painter.set_clip_rect(background.intersect(ctx.content_rect()));
                            painter.rect_filled(background, 0.0, foundation_block_bar());
                            if let Some(icon) =
                                app_icon(ctx, Path::new(self.path.trim().trim_matches('"')))
                            {
                                ui.image((icon.id(), Vec2::splat(32.0)));
                            } else {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                                paint_button_icon_at(
                                    ui,
                                    if internal {
                                        ButtonIcon::Git
                                    } else {
                                        ButtonIcon::Pin
                                    },
                                    rect,
                                    text_dark(),
                                );
                            }
                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if self.name.trim().is_empty() {
                                        "New Shortcut"
                                    } else {
                                        &self.name
                                    })
                                    .strong(),
                                )
                                .truncate(),
                            );
                            background
                        },
                    )
                    .inner;
                ui.add_space(14.0);
                ui.label(RichText::new("App Name").strong());
                ui.add_sized(
                    [ui.available_width(), BUTTON_HEIGHT],
                    crate::app::ui_kit::singleline_text_edit(&mut self.name),
                );
                if !internal {
                    ui.add_space(8.0);
                    ui.label(RichText::new("App Path").strong());
                    let previous_path = self.path.clone();
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if icon_text_button(ui, ButtonIcon::Browse, "Browse", true).clicked() {
                                let mut dialog =
                                    rfd::FileDialog::new().set_title("Select Application");
                                let current_path = Path::new(self.path.trim().trim_matches('"'));
                                let directory = if editing || !current_path.as_os_str().is_empty() {
                                    current_path.parent()
                                } else {
                                    self.browse_root.as_deref()
                                };
                                // Without an available starting folder, let the native picker
                                // use its remembered location.
                                if let Some(directory) = directory.filter(|path| path.is_dir()) {
                                    dialog = dialog.set_directory(directory);
                                }
                                #[cfg(windows)]
                                let dialog = dialog.add_filter("Application", &["exe"]);
                                if let Some(path) = dialog.pick_file() {
                                    self.select_path(path);
                                }
                            }
                            ui.add_sized(
                                [ui.available_width(), BUTTON_HEIGHT],
                                crate::app::ui_kit::singleline_text_edit(&mut self.path),
                            );
                        },
                    );
                    if self.path != previous_path
                        && crate::core::app_shortcut::validate_app_path(Path::new(self.path.trim().trim_matches('"'))).is_ok()
                    {
                        self.name = app_name(Path::new(self.path.trim().trim_matches('"')));
                        self.error = None;
                    }
                }
                ui.add_space(10.0);
                let global_changed = ui.baboon_checkbox(&mut self.global, "Global Shortcut")
                    .on_hover_text(
                        "Shortcuts this app for all existing Editing Kits, and for any newly created Editing Kit profiles.",
                    ).changed();
                if global_changed {
                    self.remove_from_other_kits = None;
                    if !self.global && self.editing.as_deref().is_some_and(|key| global_shortcut(&cx.model.prefs, &self.kit, key).is_some()) {
                        self.confirmation = Some(ShortcutScopePrompt::MakeLocal);
                    }
                }
                if let Some(error) = &self.error {
                    ui.add_space(6.0);
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.add_space(12.0);
                let footer = ui
                    .allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), 40.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let background = ui.max_rect().expand2(Vec2::new(20.0, 0.0));
                            let mut painter = ui.painter().clone();
                            painter.set_clip_rect(background.intersect(ctx.content_rect()));
                            let mut rounding = ui.visuals().window_corner_radius;
                            rounding.nw = 0;
                            rounding.ne = 0;
                            painter.rect_filled(background, rounding, foundation_block_bar());
                            if editing && !internal {
                                remove = ui.button("Remove Shortcut").clicked();
                            }
                            cancel = ui.button("Cancel").clicked();
                            save = if editing {
                                ui.button("Save").clicked()
                            } else {
                                icon_text_button(ui, ButtonIcon::Add, "Add Shortcut", true).clicked()
                            };
                            background
                        },
                    )
                    .inner;
                (banner, footer)
            });
        #[cfg(test)]
        if let Some(window) = _window {
            if let Some((banner, footer)) = window.inner {
                ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::unique("shortcut_dialog_geometry"),
                        (window.response.rect, banner, footer),
                    )
                });
            }
        }
        if cancel {
            return false;
        }
        if remove {
            if let Some(key) = self.editing.clone() {
                if global_shortcut(&cx.model.prefs, &self.kit, &key).is_some() {
                    self.confirmation = Some(ShortcutScopePrompt::Remove { from_editor: true });
                    return true;
                }
                let kit = self.kit.clone();
                cx.edit_prefs(move |prefs| remove_shortcut(prefs, &kit, &key));
            }
            return false;
        }
        if save {
            if !self.global
                && self.remove_from_other_kits.is_none()
                && self
                    .editing
                    .as_deref()
                    .is_some_and(|key| global_shortcut(&cx.model.prefs, &self.kit, key).is_some())
            {
                self.confirmation = Some(ShortcutScopePrompt::MakeLocal);
                return true;
            }
            match self.shortcut() {
                Ok(mut shortcut) => {
                    if self.builtin_default_path.as_ref() == Some(&shortcut.path) {
                        shortcut.path = PathBuf::new();
                    }
                    let editing = self.editing.clone();
                    let kit = self.kit.clone();
                    let global = self.global;
                    let open_kits = open_shortcut_kits(cx.model);
                    let remove_others = self.remove_from_other_kits.unwrap_or(false);
                    cx.edit_prefs(move |prefs| {
                        if !global
                            && let Some(key) = editing.as_deref()
                            && global_shortcut(prefs, &kit, key).is_some()
                        {
                            make_shortcut_local(
                                prefs,
                                key,
                                &kit,
                                shortcut,
                                &open_kits,
                                remove_others,
                            );
                            return;
                        }
                        save_shortcut(
                            prefs,
                            editing.as_deref(),
                            &kit,
                            global,
                            shortcut,
                            &open_kits,
                        )
                    });
                    return false;
                }
                Err(error) => self.error = Some(error),
            }
        }
        true
    }
}

fn app_name(path: &Path) -> String {
    if let Some(name) = crate::core::app_shortcut::display_name(path) {
        return name;
    }
    #[cfg(windows)]
    if let Some(name) = windows_app_name(path) {
        return name;
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut chars = stem.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

#[cfg(windows)]
fn windows_app_name(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{Win32::Storage::FileSystem::*, core::PCWSTR};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, data.as_mut_ptr().cast()).ok()?;
        let query = |key: &str| -> Option<(*mut std::ffi::c_void, u32)> {
            let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
            let mut value = std::ptr::null_mut();
            let mut len = 0;
            VerQueryValueW(
                data.as_ptr().cast(),
                PCWSTR(key.as_ptr()),
                &mut value,
                &mut len,
            )
            .as_bool()
            .then_some((value, len))
        };
        let (translations, bytes) = query("\\VarFileInfo\\Translation")?;
        for pair in std::slice::from_raw_parts(translations.cast::<u16>(), bytes as usize / 2)
            .chunks_exact(2)
        {
            for field in ["FileDescription", "ProductName"] {
                if let Some((value, len)) = query(&format!(
                    "\\StringFileInfo\\{:04x}{:04x}\\{field}",
                    pair[0], pair[1]
                )) {
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(
                        value.cast::<u16>(),
                        len as usize,
                    ))
                    .trim_end_matches('\0')
                    .trim()
                    .to_owned();
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }
        None
    }
}

/// Cache successes and failures, avoiding repeated filesystem/native calls while drawing.
pub(in crate::app) fn app_icon(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let id = egui::Id::unique(("app_shortcut_icon", path));
    if let Some(cached) = ctx.data_mut(|data| data.get_temp::<Option<egui::TextureHandle>>(id)) {
        return cached;
    }
    let image = native_icon(path).or_else(|| {
        let bytes = fs::read(path.with_extension("ico")).ok()?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Ico)
            .ok()?
            .into_rgba8();
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ))
    });
    let texture = image.map(|image| {
        ctx.load_texture(
            format!("app_icon_{}", path.display()),
            image,
            egui::TextureOptions::LINEAR,
        )
    });
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

#[cfg(not(windows))]
fn native_icon(path: &Path) -> Option<egui::ColorImage> {
    use crate::core::app_shortcut::AppIcon;
    let bytes = match crate::core::app_shortcut::native_icon(path)? {
        AppIcon::File(path) => {
            let bytes = fs::read(&path).ok()?;
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
            {
                return egui_extras::image::load_svg_bytes_with_size(
                    &bytes,
                    egui::load::SizeHint::Width(64),
                    &Default::default(),
                )
                .ok();
            }
            bytes
        }
        #[cfg(target_os = "macos")]
        AppIcon::Png(bytes) => bytes,
    };
    let image = image::load_from_memory(&bytes).ok()?.into_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

#[cfg(windows)]
fn native_icon(path: &Path) -> Option<egui::ColorImage> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Graphics::Gdi::*,
            UI::{Shell::ExtractIconExW, WindowsAndMessaging::*},
        },
        core::PCWSTR,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // Every handle returned by ExtractIconEx/GetIconInfo is released, including failures.
    unsafe {
        let mut icon = HICON::default();
        ExtractIconExW(PCWSTR(wide.as_ptr()), 0, Some(&mut icon), None, 1);
        if icon.is_invalid() {
            return None;
        }
        let mut info = ICONINFO::default();
        let result = if GetIconInfo(icon, &mut info).is_ok() {
            (|| {
                let mut bitmap = BITMAP::default();
                if GetObjectW(
                    HGDIOBJ(info.hbmColor.0),
                    size_of::<BITMAP>() as i32,
                    Some((&mut bitmap as *mut BITMAP).cast()),
                ) == 0
                {
                    return None;
                }
                let (width, height) = (bitmap.bmWidth, bitmap.bmHeight.abs());
                if width <= 0 || height <= 0 || width > 1024 || height > 1024 {
                    return None;
                }
                let mut header = BITMAPINFO::default();
                header.bmiHeader = BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                };
                let dc = GetDC(None);
                if dc.is_invalid() {
                    return None;
                }
                let mut pixels = vec![0u8; width as usize * height as usize * 4];
                let lines = GetDIBits(
                    dc,
                    info.hbmColor,
                    0,
                    height as u32,
                    Some(pixels.as_mut_ptr().cast()),
                    &mut header,
                    DIB_RGB_COLORS,
                );
                // Older icons carry transparency in their AND mask rather than an alpha channel.
                if pixels.chunks_exact(4).all(|p| p[3] == 0) {
                    let mut mask = vec![0u8; pixels.len()];
                    if GetDIBits(
                        dc,
                        info.hbmMask,
                        0,
                        height as u32,
                        Some(mask.as_mut_ptr().cast()),
                        &mut header,
                        DIB_RGB_COLORS,
                    ) == height
                    {
                        for (pixel, mask) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                            pixel[3] = if mask[0] == 0 { 255 } else { 0 };
                        }
                    }
                }
                ReleaseDC(None, dc);
                if lines != height {
                    return None;
                }
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                Some(egui::ColorImage::from_rgba_premultiplied(
                    [width as usize, height as usize],
                    &pixels,
                ))
            })()
        } else {
            None
        };
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        }
        let _ = DestroyIcon(icon);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_tool_context_menus_allow_edits_while_git_review_stays_fixed() {
        for row in 0..3 {
            let mut app = Baboon::for_test();
            let ctx = egui::Context::default();
            let rects = shortcut_frame(&mut app, &ctx, Vec::new());
            let pos = rects[row].center();
            let pointer = |pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Secondary,
                modifiers: Default::default(),
            };
            shortcut_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(pos), pointer(true)],
            );
            shortcut_frame(&mut app, &ctx, vec![pointer(false)]);
            shortcut_frame(&mut app, &ctx, Vec::new());
            let texts = ctx
                .data(|data| {
                    data.get_temp::<Vec<(String, egui::Rect)>>(egui::Id::unique("shortcut_test_texts"))
                })
                .unwrap();
            let editable = row != 0;
            assert_eq!(
                texts.iter().any(|(text, _)| text == "Edit Shortcut..."),
                editable
            );
            assert_eq!(
                texts.iter().any(|(text, _)| text == "Remove Shortcut"),
                editable
            );
            if editable {
                let edit = texts
                    .iter()
                    .find(|(text, _)| text == "Edit Shortcut...")
                    .unwrap()
                    .1
                    .center();
                let click = |pressed| egui::Event::PointerButton {
                    pos: edit,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                };
                shortcut_frame(
                    &mut app,
                    &ctx,
                    vec![egui::Event::PointerMoved(edit), click(true)],
                );
                shortcut_frame(&mut app, &ctx, vec![click(false)]);
                let draft = app.dialogs.get::<AddShortcutDialog>().unwrap();
                assert_eq!(
                    draft.editing.as_deref(),
                    Some(if row == 1 {
                        "builtin:tag_test"
                    } else {
                        "builtin:sapien"
                    })
                );
                assert!(!draft.global);
            } else {
                assert!(app.dialogs.get::<AddShortcutDialog>().is_none());
            }
        }
    }

    #[test]
    fn shortcut_scopes_separate_app_versions_and_share_only_globals() {
        let mut prefs = GuiPrefs::default();
        let app = |name: &str, path: &str| AppShortcut {
            id: new_shortcut_id(),
            kit: None,
            name: name.to_owned(),
            path: PathBuf::from(path),
        };
        save_shortcut(
            &mut prefs,
            None,
            "kit-one",
            false,
            app("Blender 4.3", "blender43.exe"),
            &[],
        );
        save_shortcut(
            &mut prefs,
            None,
            "kit-two",
            false,
            app("Blender 5.1", "blender51.exe"),
            &[],
        );
        save_shortcut(
            &mut prefs,
            None,
            "kit-one",
            true,
            app("Shared Tool", "shared.exe"),
            &[],
        );
        let (one, two, global) = (
            prefs.app_shortcuts[0].key(),
            prefs.app_shortcuts[1].key(),
            prefs.app_shortcuts[2].key(),
        );
        assert!(shortcut_keys(&prefs, "kit-one").contains(&one));
        assert!(!shortcut_keys(&prefs, "kit-one").contains(&two));
        assert!(shortcut_keys(&prefs, "kit-two").contains(&two));
        assert!(!shortcut_keys(&prefs, "kit-two").contains(&one));
        for kit in ["kit-one", "kit-two"] {
            assert!(shortcut_keys(&prefs, kit).contains(&global));
        }
        let existing = prefs.app_shortcuts[0].clone();
        save_shortcut(&mut prefs, Some(&one), "kit-one", true, existing, &[]);
        assert!(shortcut_keys(&prefs, "kit-two").contains(&one));
        let mut existing = prefs.app_shortcuts[0].clone();
        existing.name = "Edited Blender".to_owned();
        existing.path = PathBuf::from("edited.exe");
        save_shortcut(&mut prefs, Some(&one), "kit-one", false, existing, &[]);
        assert!(shortcut_keys(&prefs, "kit-one").contains(&one));
        assert!(!shortcut_keys(&prefs, "kit-two").contains(&one));
        let local = prefs
            .app_shortcuts
            .iter()
            .find(|app| app.key() == one)
            .unwrap();
        assert_eq!(local.name, "Edited Blender");
        assert_eq!(local.path, Path::new("edited.exe"));
        assert_eq!(local.kit.as_deref(), Some("kit-one"));
        assert!(shortcut_keys(&prefs, "kit-one").contains(&local.key()));
        assert!(!shortcut_keys(&prefs, "kit-two").contains(&local.key()));
        let other = prefs
            .app_shortcuts
            .iter()
            .find(|app| app.name == "Blender 4.3")
            .unwrap();
        assert_eq!(other.kit.as_deref(), Some("kit-two"));
        assert_eq!(other.path, Path::new("blender43.exe"));
        let other_key = other.key();
        remove_shortcut(&mut prefs, "kit-two", &other_key);
        assert!(!shortcut_keys(&prefs, "kit-two").contains(&other_key));
        assert!(shortcut_keys(&prefs, "kit-one").contains(&one));
        assert!(!shortcut_keys(&prefs, "kit-three").contains(&one));
        let app = Baboon::for_test();
        let draft = AddShortcutDialog::for_kit(&app.model, 0);
        assert!(!draft.global);
        assert_eq!(draft.kit, shortcut_kit_key(&app.model, 0));
    }

    #[test]
    fn making_an_app_global_rejoins_local_copies_and_allows_removing_them_together() {
        for adding in [false, true] {
            let mut prefs = GuiPrefs::default();
            let shared = AppShortcut {
                id: new_shortcut_id(), kit: None,
                name: "Blender 4.3".to_owned(), path: PathBuf::from("blender43.exe"),
            };
            let key = shared.key();
            save_shortcut(&mut prefs, None, "kit-one", true, shared.clone(), &[]);
            make_shortcut_local(&mut prefs, &key, "kit-one", shared.clone(),
                &["kit-two".to_owned(), "kit-three".to_owned()], false);
            let copies = prefs.app_shortcuts.clone();
            assert_eq!(copies.len(), 3);
            for copy in &copies {
                let layout = local_layout(&mut prefs, copy.kit.as_deref().unwrap());
                layout.order = vec!["builtin:sapien".to_owned(), copy.key(), "builtin:tag_test".to_owned()];
            }
            // Display-name changes still describe the same app; a different executable does not.
            prefs.app_shortcuts[1].name = "My Blender".to_owned();
            let other_version = AppShortcut {
                id: new_shortcut_id(), kit: Some("kit-two".to_owned()),
                name: "Blender 4.3".to_owned(), path: PathBuf::from("blender51.exe"),
            };
            prefs.app_shortcuts.push(other_version.clone());
            let mut promoted = shared;
            if adding { promoted.id = new_shortcut_id(); }
            save_shortcut(&mut prefs, (!adding).then_some(key.as_str()), "kit-one", true, promoted, &[]);
            assert_eq!(prefs.app_shortcuts.len(), 2);
            let global = prefs.app_shortcuts.iter().find(|app| app.kit.is_none()).unwrap().clone();
            assert_eq!(global.key(), key);
            assert!(prefs.app_shortcuts.contains(&other_version));
            for kit in ["kit-one", "kit-two", "kit-three", "new-kit"] {
                assert!(global_shortcut(&prefs, kit, &key).is_some());
                let keys = shortcut_keys(&prefs, kit);
                assert_eq!(keys.iter().filter(|entry| *entry == &key).count(), 1);
                if kit != "new-kit" {
                    assert_eq!(keys[1], key, "keep the former copy's position");
                }
                for old in &copies {
                    if old.key() != key { assert!(!keys.contains(&old.key())); }
                }
            }
            make_shortcut_local(&mut prefs, &key, "kit-one", global, &[], true);
            remove_shortcut(&mut prefs, "kit-one", &key);
            assert_eq!(prefs.app_shortcuts, vec![other_version]);
        }
    }

    #[test]
    fn global_app_addition_reuses_existing_global_and_restores_hidden_copies() {
        let mut prefs = GuiPrefs::default();
        let shared = AppShortcut {
            id: new_shortcut_id(), kit: None,
            name: "Tool".to_owned(), path: PathBuf::from("tool.exe"),
        };
        let key = shared.key();
        save_shortcut(&mut prefs, None, "kit-one", true, shared.clone(), &[]);
        remove_shortcut(&mut prefs, "kit-two", &key);
        prefs.hidden_shortcuts.insert(key.clone());
        let mut duplicate = shared;
        duplicate.id = new_shortcut_id();
        duplicate.name = "Renamed Tool".to_owned();
        save_shortcut(&mut prefs, None, "kit-one", true, duplicate, &[]);
        assert_eq!(prefs.app_shortcuts.len(), 1);
        assert_eq!(prefs.app_shortcuts[0].key(), key);
        assert_eq!(prefs.app_shortcuts[0].name, "Renamed Tool");
        assert!(shortcut_keys(&prefs, "kit-two").contains(&key));
        assert!(shortcut_keys(&prefs, "new-kit").contains(&key));
    }

    #[test]
    fn making_a_default_tool_global_rejoins_matching_local_overrides() {
        let mut prefs = GuiPrefs::default();
        let key = "builtin:sapien";
        let shared = AppShortcut {
            id: key.to_owned(), kit: None,
            name: "Sapien".to_owned(), path: PathBuf::from("sapien.exe"),
        };
        save_shortcut(&mut prefs, Some(key), "kit-one", true, shared.clone(), &[]);
        make_shortcut_local(&mut prefs, key, "kit-one", shared.clone(),
            &["kit-two".to_owned(), "custom-kit".to_owned()], false);
        local_layout(&mut prefs, "custom-kit").overrides.get_mut(key).unwrap().path = PathBuf::from("custom-sapien.exe");
        save_shortcut(&mut prefs, Some(key), "kit-one", true, shared, &[]);
        for kit in ["kit-one", "kit-two", "new-kit"] {
            assert!(global_shortcut(&prefs, kit, key).is_some());
            assert!(!prefs.kit_shortcut_layouts.get(kit).is_some_and(|layout| layout.overrides.contains_key(key)));
        }
        assert!(global_shortcut(&prefs, "custom-kit", key).is_none());
        assert_eq!(builtin_shortcut(&prefs, "custom-kit", key, None, "tag_test.exe").path, Path::new("custom-sapien.exe"));
    }

    #[test]
    fn turning_off_global_keeps_existing_kits_local_without_restoring_removed_shortcuts() {
        let mut prefs = GuiPrefs::default();
        prefs
            .custom_editing_kit_profiles
            .push(CustomEditingKitProfile {
                recovery: Default::default(),
                id: "closed-kit".to_owned(),
                name: "Closed kit".to_owned(),
                game: "halo3_mcc".to_owned(),
                root: PathBuf::from("closed-kit"),
                read_only: false,
                git_tracked: false,
                icon: None,
                tags_folder: None,
                data_folder: None,
            });
        let shared = AppShortcut {
            id: new_shortcut_id(),
            kit: None,
            name: "Blender".to_owned(),
            path: PathBuf::from("blender.exe"),
        };
        let key = shared.key();
        prefs.app_shortcuts.push(shared.clone());
        reorder_shortcut(
            &mut prefs,
            "other-open-kit",
            &key,
            "builtin:git_review",
            false,
        );
        remove_shortcut(&mut prefs, "removed-kit", &key);
        let mut edited = shared;
        edited.name = "My Blender".to_owned();
        save_shortcut(
            &mut prefs,
            Some(&key),
            "current-kit",
            false,
            edited,
            &[
                "other-open-kit".to_owned(),
                "unmodified-open-kit".to_owned(),
            ],
        );

        // The persisted copies retain their scopes across a restart.
        prefs.app_shortcuts =
            serde_json::from_value(serde_json::to_value(&prefs.app_shortcuts).unwrap()).unwrap();
        prefs.kit_shortcut_layouts =
            serde_json::from_value(serde_json::to_value(&prefs.kit_shortcut_layouts).unwrap())
                .unwrap();
        assert_eq!(prefs.app_shortcuts.len(), 4);
        let mut ids = HashSet::new();
        for kit in [
            "current-kit",
            "other-open-kit",
            "unmodified-open-kit",
            "profile:closed-kit",
        ] {
            let local = prefs
                .app_shortcuts
                .iter()
                .find(|app| app.kit.as_deref() == Some(kit))
                .unwrap();
            assert!(ids.insert(local.id.clone()));
            assert!(shortcut_keys(&prefs, kit).contains(&local.key()));
            assert_eq!(
                local.name,
                if kit == "current-kit" {
                    "My Blender"
                } else {
                    "Blender"
                }
            );
        }
        let other = prefs
            .app_shortcuts
            .iter()
            .find(|app| app.kit.as_deref() == Some("other-open-kit"))
            .unwrap();
        assert_eq!(shortcut_keys(&prefs, "other-open-kit")[0], other.key());
        let other_key = other.key();
        remove_shortcut(&mut prefs, "other-open-kit", &other_key);
        assert!(shortcut_keys(&prefs, "current-kit").contains(&key));
        assert!(prefs.app_shortcuts.iter().all(|app| app.kit.is_some()));
        for kit in ["removed-kit", "brand-new-kit"] {
            assert!(
                shortcut_keys(&prefs, kit)
                    .iter()
                    .all(|key| !key.starts_with("app:"))
            );
        }
    }

    #[test]
    fn turning_off_global_tool_override_localizes_it_for_existing_kits() {
        let key = "builtin:tag_test";
        let mut prefs = GuiPrefs::default();
        let shared = AppShortcut {
            id: key.to_owned(),
            kit: None,
            name: "Shared Test".to_owned(),
            path: PathBuf::from("shared.exe"),
        };
        prefs
            .shortcut_overrides
            .insert(key.to_owned(), shared.clone());
        let mut custom = shared.clone();
        custom.name = "Existing local edit".to_owned();
        save_shortcut(
            &mut prefs,
            Some(key),
            "locally-edited-kit",
            false,
            custom,
            &[],
        );
        // Restore a global definition to exercise a kit that already has a local override.
        prefs
            .shortcut_overrides
            .insert(key.to_owned(), shared.clone());
        let local = builtin_shortcut(&prefs, "locally-edited-kit", key, None, "tag_test.exe");
        save_shortcut(
            &mut prefs,
            Some(key),
            "locally-edited-kit",
            false,
            local,
            &[],
        );
        assert!(prefs.shortcut_overrides.contains_key(key));
        remove_shortcut(&mut prefs, "removed-kit", key);
        let mut edited = shared;
        edited.name = "My Test".to_owned();
        save_shortcut(
            &mut prefs,
            Some(key),
            "current-kit",
            false,
            edited,
            &["other-kit".to_owned()],
        );
        assert!(!prefs.shortcut_overrides.contains_key(key));
        for (kit, expected) in [
            ("current-kit", "My Test"),
            ("other-kit", "Shared Test"),
            ("locally-edited-kit", "Existing local edit"),
        ] {
            let local = builtin_shortcut(&prefs, kit, key, None, "tag_test.exe");
            assert_eq!(local.name, expected);
            assert_eq!(local.kit.as_deref(), Some(kit));
        }
        assert!(!shortcut_keys(&prefs, "removed-kit").contains(&key.to_owned()));
        assert_eq!(
            builtin_shortcut(&prefs, "brand-new-kit", key, None, "tag_test.exe").name,
            "Tag Test"
        );
    }

    #[test]
    fn shortcut_tool_edits_and_removals_are_kit_specific() {
        let mut prefs = GuiPrefs::default();
        let original = shortcut_keys(&prefs, "kit-two");
        reorder_shortcut(
            &mut prefs,
            "kit-one",
            "builtin:sapien",
            "builtin:git_review",
            false,
        );
        assert_eq!(shortcut_keys(&prefs, "kit-two"), original);
        assert_ne!(shortcut_keys(&prefs, "kit-one"), original);
        remove_shortcut(&mut prefs, "kit-one", "builtin:sapien");
        assert!(shortcut_keys(&prefs, "kit-two").contains(&"builtin:sapien".to_owned()));
        let app = AppShortcut {
            id: new_shortcut_id(),
            kit: None,
            name: "Custom Test".to_owned(),
            path: PathBuf::from("custom.exe"),
        };
        save_shortcut(
            &mut prefs,
            Some("builtin:tag_test"),
            "kit-one",
            false,
            app,
            &[],
        );
        assert_eq!(
            builtin_shortcut(&prefs, "kit-one", "builtin:tag_test", None, "tag_test.exe").path,
            Path::new("custom.exe")
        );
        assert_eq!(
            builtin_shortcut(
                &prefs,
                "kit-two",
                "builtin:tag_test",
                Some(Path::new("kit-two")),
                "tag_test.exe"
            )
            .path,
            Path::new("kit-two").join("tag_test.exe")
        );
    }

    #[test]
    fn shortcut_dialog_hugs_content_with_padded_banner_and_full_width_footer() {
        for scale in [1.0, 1.5] {
            for editing in [false, true] {
                let ctx = egui::Context::default();
                Baboon::configure_context(&ctx);
                ctx.set_pixels_per_point(scale);
                let app = Baboon::for_test();
                let (jobs, _) = mpsc::channel();
                let reads = app_reads!(app);
                let mut draft = AddShortcutDialog::for_kit(&app.model, 0);
                draft.name = "Blender 4.3".to_owned();
                if editing {
                    draft.editing = Some("app:existing".to_owned());
                }
                let frame = |draft: &mut AddShortcutDialog| {
                    let output = crate::app::run_ui_test(
                        &ctx,
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(900.0, 700.0),
                            )),
                            ..Default::default()
                        },
                        |_| {
                            let cx = Ctx::new(&app.model, &ctx, &app.commands, &jobs);
                            draft.show(&cx, &reads);
                        },
                    );
                    let geometry = ctx
                        .data(|data| {
                            data.get_temp::<(egui::Rect, egui::Rect, egui::Rect)>(egui::Id::unique(
                                "shortcut_dialog_geometry",
                            ))
                        })
                        .unwrap();
                    (geometry, output)
                };
                frame(&mut draft);
                let ((window, banner, footer), output) = frame(&mut draft);
                let labels: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some((text.galley.text(), text.pos.x)),
                        _ => None,
                    })
                    .collect();
                assert!(labels.iter().any(|(text, _)| *text == "Global Shortcut"));
                assert!(!labels.iter().any(|(text, _)| *text == "Save Changes"));
                if editing {
                    let position =
                        |label| labels.iter().find(|(text, _)| *text == label).unwrap().1;
                    assert!(position("Save") < position("Cancel"));
                    assert!(position("Cancel") < position("Remove Shortcut"));
                } else {
                    assert!(!labels.iter().any(|(text, _)| *text == "Remove Shortcut"));
                }
                assert!(
                    window.height() < 360.0,
                    "window should fit its contents: {window:?}"
                );
                assert!((banner.left() - window.left() - 8.0).abs() <= 2.0);
                assert!((window.right() - banner.right() - 8.0).abs() <= 2.0);
                assert!((footer.left() - window.left()).abs() <= 2.0);
                assert!((footer.right() - window.right()).abs() <= 2.0);
                assert!((footer.bottom() - window.bottom()).abs() <= 2.0);
                let ((next, _, _), _) = frame(&mut draft);
                assert!(
                    (next.width() - window.width()).abs() <= 1.0,
                    "dialog must not grow each frame"
                );
            }
        }
    }

    #[test]
    fn edit_shortcut_remove_button_removes_only_the_current_kits_copy() {
        for builtin in [false, true] {
            let mut app = Baboon::for_test();
            let shared = AppShortcut {
                id: new_shortcut_id(),
                kit: None,
                name: "Shared App".to_owned(),
                path: PathBuf::from("missing-app.exe"),
            };
            let key = if builtin {
                "builtin:sapien".to_owned()
            } else {
                shared.key()
            };
            app.model.prefs.app_shortcuts.push(shared);
            local_layout(&mut app.model.prefs, "other-kit");
            let mut dialog = AddShortcutDialog::for_kit(&app.model, 0);
            let kit = dialog.kit.clone();
            dialog.editing = Some(key.clone());
            dialog.name = "An unavailable app".to_owned();
            app.dialogs.open(dialog);
            let mut driver = crate::app::loose_fixture::PromptDriver::new();
            driver.click(&mut app, "Remove Shortcut");
            if !builtin {
                assert!(
                    app.dialogs
                        .get::<AddShortcutDialog>()
                        .unwrap()
                        .confirmation
                        .is_some()
                );
                driver.click(&mut app, "Keep Others as Copies");
            }
            assert!(app.dialogs.get::<AddShortcutDialog>().is_none());
            assert!(!shortcut_keys(&app.model.prefs, &kit).contains(&key));
            if builtin {
                assert!(shortcut_keys(&app.model.prefs, "other-kit").contains(&key));
            } else {
                let other = app
                    .model
                    .prefs
                    .app_shortcuts
                    .iter()
                    .find(|app| app.kit.as_deref() == Some("other-kit"))
                    .unwrap();
                assert!(shortcut_keys(&app.model.prefs, "other-kit").contains(&other.key()));
            }
        }
    }

    #[test]
    fn global_shortcut_prompts_cover_removal_unmarking_and_cancellation() {
        for builtin in [false, true] {
            for action in ["context_remove", "editor_remove", "make_local"] {
                for choice in ["Cancel", "Keep Others as Copies", "Remove From Other Kits"] {
                    let mut app = Baboon::for_test();
                    let shared = AppShortcut {
                        id: new_shortcut_id(),
                        kit: None,
                        name: "Global App".to_owned(),
                        path: std::env::current_exe().unwrap(),
                    };
                    let key = if builtin {
                        "builtin:sapien".to_owned()
                    } else {
                        shared.key()
                    };
                    if builtin {
                        app.model
                            .prefs
                            .shortcut_overrides
                            .insert(key.clone(), shared.clone());
                    } else {
                        app.model.prefs.app_shortcuts.push(shared.clone());
                    }
                    local_layout(&mut app.model.prefs, "other-kit");
                    let kit = shortcut_kit_key(&app.model, 0);
                    let before = app.model.prefs.clone();
                    let mut dialog = if action == "context_remove" {
                        AddShortcutDialog::for_global_removal(&app.model, 0, &key).unwrap()
                    } else {
                        AddShortcutDialog::for_kit(&app.model, 0)
                    };
                    dialog.editing = Some(key.clone());
                    dialog.global = true;
                    dialog.name = shared.name.clone();
                    dialog.path = shared.path.display().to_string();
                    app.dialogs.open(dialog);
                    let mut driver = crate::app::loose_fixture::PromptDriver::new();
                    if action != "context_remove" {
                        driver.click(
                            &mut app,
                            if action == "make_local" {
                                "Global Shortcut"
                            } else {
                                "Remove Shortcut"
                            },
                        );
                    }
                    assert!(
                        app.model.prefs == before,
                        "prompt must not mutate preferences"
                    );
                    driver.frame(&mut app, Vec::new()); // Let a newly opened modal finish sizing.
                    let labels = driver.frame(&mut app, Vec::new());
                    assert!(
                        labels.iter().any(|(label, _)| label == "Remove Global Shortcut"),
                        "missing prompt: builtin={builtin}, action={action}, choice={choice}, labels={labels:?}"
                    );
                    driver.click(&mut app, choice);
                    if choice == "Cancel" {
                        assert!(app.model.prefs == before);
                        if action == "context_remove" {
                            assert!(app.dialogs.get::<AddShortcutDialog>().is_none());
                        } else {
                            let dialog = app.dialogs.get::<AddShortcutDialog>().unwrap();
                            assert!(dialog.global);
                            assert!(dialog.confirmation.is_none());
                        }
                        continue;
                    }
                    if action == "make_local" {
                        assert!(
                            app.model.prefs == before,
                            "unmarking is committed only on Save"
                        );
                        driver.click(&mut app, "Save");
                    }
                    assert!(app.dialogs.get::<AddShortcutDialog>().is_none());
                    assert!(global_shortcut(&app.model.prefs, &kit, &key).is_none());
                    assert_eq!(
                        shortcut_keys(&app.model.prefs, &kit).contains(&key),
                        action == "make_local"
                    );
                    if builtin {
                        assert_eq!(
                            shortcut_keys(&app.model.prefs, "other-kit").contains(&key),
                            choice == "Keep Others as Copies"
                        );
                        assert_eq!(
                            builtin_shortcut(
                                &app.model.prefs,
                                "new-kit",
                                &key,
                                None,
                                "tag_test.exe"
                            )
                            .name,
                            "Sapien"
                        );
                    } else {
                        let copies = &app.model.prefs.app_shortcuts;
                        assert_eq!(
                            copies
                                .iter()
                                .any(|app| app.kit.as_deref() == Some("other-kit")),
                            choice == "Keep Others as Copies"
                        );
                        assert!(copies.iter().all(|app| app.kit.is_some()));
                        assert!(
                            shortcut_keys(&app.model.prefs, "new-kit")
                                .iter()
                                .all(|key| !key.starts_with("app:"))
                        );
                    }
                }
            }
        }
    }

    fn shortcut_frame(
        app: &mut Baboon,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::Rect> {
        let (jobs, _) = mpsc::channel();
        let mut rects = Vec::new();
        let output = crate::app::run_ui_test(
            ctx,
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let cx = Ctx::new(&app.model, ctx, &app.commands, &jobs);
                    let area = ui.scope_id();
                    for key in shortcut_keys(&app.model.prefs, "workspace:0") {
                        ui.push_id(&key, |ui| {
                            let response = ui.add_sized(
                                [300.0, 24.0],
                                egui::Button::new(&key).sense(Sense::click_and_drag()),
                            );
                            rects.push(response.rect);
                            shortcut_interactions(&cx, ui, &response, &key, area, None, 0);
                        });
                    }
                });
            },
        );
        let text_rects: Vec<(String, egui::Rect)> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    ))
                } else {
                    None
                }
            })
            .collect();
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique("shortcut_test_texts"), text_rects));
        app.apply_commands(ctx);
        rects
    }

    #[test]
    fn shortcut_drag_reorders_inside_list_and_ignores_outside_drop() {
        let mut app = Baboon::for_test();
        let ctx = egui::Context::default();
        let rects = shortcut_frame(&mut app, &ctx, Vec::new());
        let start = rects[0].center();
        let end = rects[2].center() + Vec2::new(0.0, 8.0);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        };
        shortcut_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), pointer(start, true)],
        );
        shortcut_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        shortcut_frame(&mut app, &ctx, vec![pointer(end, false)]);
        assert_eq!(
            shortcut_keys(&app.model.prefs, "workspace:0"),
            ["builtin:tag_test", "builtin:sapien", "builtin:git_review"]
        );
        let before = shortcut_keys(&app.model.prefs, "workspace:0");
        let rects = shortcut_frame(&mut app, &ctx, Vec::new());
        let start = rects[0].center();
        let end = egui::pos2(600.0, 600.0);
        shortcut_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), pointer(start, true)],
        );
        shortcut_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        shortcut_frame(&mut app, &ctx, vec![pointer(end, false)]);
        assert_eq!(shortcut_keys(&app.model.prefs, "workspace:0"), before);
    }

    #[test]
    fn shortcut_context_menu_opens_edit_dialog_and_removes_shortcut() {
        let mut app = Baboon::for_test();
        let key = "app:editable".to_owned();
        app.model.prefs.app_shortcuts.push(AppShortcut {
            id: "editable".to_owned(),
            kit: Some("workspace:0".to_owned()),
            name: "Editable".to_owned(),
            path: std::env::current_exe().unwrap(),
        });
        let ctx = egui::Context::default();
        let rects = shortcut_frame(&mut app, &ctx, Vec::new());
        let pos = rects.last().unwrap().center();
        let pointer = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        shortcut_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(pos),
                pointer(pos, egui::PointerButton::Secondary, true),
            ],
        );
        shortcut_frame(
            &mut app,
            &ctx,
            vec![pointer(pos, egui::PointerButton::Secondary, false)],
        );
        shortcut_frame(&mut app, &ctx, Vec::new());
        let label_pos = |label: &str| {
            ctx.data(|data| {
                data.get_temp::<Vec<(String, egui::Rect)>>(egui::Id::unique("shortcut_test_texts"))
            })
            .unwrap()
            .into_iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("missing menu item {label}"))
            .1
            .center()
        };
        let edit = label_pos("Edit Shortcut...");
        shortcut_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(edit),
                pointer(edit, egui::PointerButton::Primary, true),
            ],
        );
        shortcut_frame(
            &mut app,
            &ctx,
            vec![pointer(edit, egui::PointerButton::Primary, false)],
        );
        assert_eq!(
            app.dialogs
                .get::<AddShortcutDialog>()
                .unwrap()
                .editing
                .as_deref(),
            Some(key.as_str())
        );
        shortcut_frame(&mut app, &ctx, Vec::new());
        shortcut_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(pos),
                pointer(pos, egui::PointerButton::Secondary, true),
            ],
        );
        shortcut_frame(
            &mut app,
            &ctx,
            vec![pointer(pos, egui::PointerButton::Secondary, false)],
        );
        shortcut_frame(&mut app, &ctx, Vec::new());
        let remove = label_pos("Remove Shortcut");
        shortcut_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(remove),
                pointer(remove, egui::PointerButton::Primary, true),
            ],
        );
        shortcut_frame(
            &mut app,
            &ctx,
            vec![pointer(remove, egui::PointerButton::Primary, false)],
        );
        assert!(!shortcut_keys(&app.model.prefs, "workspace:0").contains(&key));
    }

    #[test]
    fn shortcut_removal_and_edit_keep_stable_identity() {
        let mut prefs = GuiPrefs::default();
        let app = AppShortcut {
            id: new_shortcut_id(),
            kit: Some("kit-one".to_owned()),
            name: "Before".to_owned(),
            path: PathBuf::from("app.exe"),
        };
        let key = app.key();
        prefs.app_shortcuts.push(app.clone());
        reorder_shortcut(&mut prefs, "kit-one", &key, "builtin:tag_test", false);
        remove_shortcut(&mut prefs, "kit-one", "builtin:sapien");
        assert!(!shortcut_keys(&prefs, "kit-one").contains(&"builtin:sapien".to_owned()));
        let draft = AddShortcutDialog {
            editing: Some(key.clone()),
            name: "After".to_owned(),
            path: std::env::current_exe().unwrap().display().to_string(),
            ..Default::default()
        };
        let edited = draft.shortcut().unwrap();
        assert_eq!(edited.key(), key);
        assert_eq!(edited.name, "After");
        remove_shortcut(&mut prefs, "kit-one", &key);
        assert!(prefs.app_shortcuts.is_empty());
        assert!(!prefs.kit_shortcut_layouts["kit-one"].order.contains(&key));
    }

    #[test]
    fn shortcut_tools_respect_legacy_overrides_and_removals() {
        let mut prefs = GuiPrefs::default();
        prefs.hidden_shortcuts.insert("builtin:tag_test".to_owned());
        local_layout(&mut prefs, "kit-one")
            .hidden
            .insert("builtin:sapien".to_owned());
        let keys = shortcut_keys(&prefs, "kit-one");
        assert!(!keys.contains(&"builtin:tag_test".to_owned()));
        assert!(!keys.contains(&"builtin:sapien".to_owned()));
        prefs.shortcut_overrides.insert(
            "builtin:tag_test".to_owned(),
            AppShortcut {
                id: new_shortcut_id(),
                kit: None,
                name: "Play".to_owned(),
                path: PathBuf::new(),
            },
        );
        let first = builtin_shortcut(
            &prefs,
            "kit-one",
            "builtin:tag_test",
            Some(Path::new("kit-one")),
            "tag_test.exe",
        );
        let second = builtin_shortcut(
            &prefs,
            "kit-one",
            "builtin:tag_test",
            Some(Path::new("kit-two")),
            "halo2_tag_test.exe",
        );
        assert_eq!(first.name, "Play");
        assert_eq!(first.path, Path::new("kit-one").join("tag_test.exe"));
        assert_eq!(second.path, Path::new("kit-two").join("halo2_tag_test.exe"));
    }

    #[test]
    fn shortcut_path_autofills_name_and_keeps_display_name_editable() {
        let path = std::env::temp_dir().join(format!(
            "shortcut-test-{}{}",
            uuid::Uuid::new_v4(),
            if cfg!(windows) { ".exe" } else { "" }
        ));
        fs::write(&path, []).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut draft = AddShortcutDialog::default();
        draft.select_path(path.clone());
        assert_eq!(draft.name, app_name(&path));
        draft.name = "  My Custom App  ".to_owned();
        let shortcut = draft.shortcut().unwrap();
        assert_eq!(shortcut.name, "My Custom App");
        assert_eq!(shortcut.path, path);
        draft.name.clear();
        assert!(draft.shortcut().is_err());
        fs::remove_file(&path).unwrap();
        draft.name = "Missing app".to_owned();
        assert!(draft.shortcut().is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_shortcut_reads_executable_icon_and_product_name() {
        let executable =
            PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32/notepad.exe");
        assert!(!windows_app_name(&executable).unwrap().is_empty());
        let ctx = egui::Context::default();
        let icon = app_icon(&ctx, &executable).expect("Windows executable icon");
        assert_eq!(icon.id(), app_icon(&ctx, &executable).unwrap().id());
        assert!(icon.size()[0] > 0);
    }
}
