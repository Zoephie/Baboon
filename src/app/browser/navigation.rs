//! Keyboard selection follows displayed folder/tag order, not source order.
use super::*;

#[derive(Clone)]
pub(super) struct NavigationFolder {
    pub path: PathBuf,
    pub label: String,
    collapse_id: Option<egui::Id>,
    open: bool,
}

enum NavigationAction {
    Select(String, egui::Id),
    Toggle(String, egui::Id, egui::Id, bool),
}

#[derive(Clone, Default)]
struct Navigation {
    scope: Option<egui::Id>,
    owner_scope: Option<egui::Id>,
    owner: Option<egui::Id>,
    rows: Vec<(String, egui::Id)>,
    ready: bool,
    collecting: bool,
    seen: HashSet<egui::Id>,
    step: Option<egui::Key>,
    columns: usize,
    page_rows: usize,
    target: Option<(String, egui::Id)>,
    parents: HashMap<egui::Id, Option<egui::Id>>,
    folders: HashMap<egui::Id, NavigationFolder>,
    parent_stack: Vec<(egui::Id, bool)>,
}

fn state_id() -> egui::Id { egui::Id::new("browser_keyboard_navigation") }

fn state<R>(ctx: &egui::Context, run: impl FnOnce(&mut Navigation) -> R) -> R {
    ctx.data_mut(|data| run(data.get_temp_mut_or_default::<Navigation>(state_id())))
}

pub(super) fn begin_browser_navigation(ui: &Ui, scope: egui::Id) {
    state(ui.ctx(), |nav| {
        nav.scope = Some(scope);
        nav.parent_stack.clear();
        nav.collecting = nav.owner_scope == Some(scope) && nav.step.is_some() && !nav.ready;
        if nav.collecting { nav.rows.clear(); nav.seen.clear(); nav.parents.clear(); nav.columns = 1; nav.page_rows = 0; }
    });
}

pub(super) fn clear_browser_navigation(ctx: &egui::Context) {
    state(ctx, |nav| *nav = Navigation::default());
}

pub(in crate::app) fn invalidate_browser_navigation(ctx: &egui::Context) {
    state(ctx, |nav| { nav.ready = false; nav.rows.clear(); nav.target = None; });
}

pub(super) fn browser_navigation_select(ui: &Ui, id: egui::Id) {
    state(ui.ctx(), |nav| {
        nav.owner_scope = nav.scope;
        nav.owner = Some(id);
        nav.ready = false;
        nav.rows.clear();
        nav.target = None;
        nav.step = None;
    });
}

pub(super) fn register_browser_navigation(ui: &Ui, key: &str, id: egui::Id) {
    state(ui.ctx(), |nav| {
        if nav.collecting && nav.parent_stack.iter().all(|(_, open)| *open) && nav.seen.insert(id) {
            if nav.page_rows == 0 {
                nav.page_rows = (ui.clip_rect().height() / (ui.spacing().interact_size.y + ui.spacing().item_spacing.y)).floor().max(1.0) as usize;
            }
            nav.rows.push((key.to_owned(), id));
            nav.parents.insert(id, nav.parent_stack.last().map(|(id, _)| *id));
        }
    });
}

pub(super) fn register_browser_folder(ui: &Ui, id: egui::Id, collapse_id: Option<egui::Id>, path: &Path, label: &str, open: bool) {
    state(ui.ctx(), |nav| { nav.folders.insert(id, NavigationFolder {
        path: path.to_owned(), label: label.to_owned(), collapse_id, open,
    }); });
}

pub(super) fn selected_browser_folder(ctx: &egui::Context) -> Option<NavigationFolder> {
    state(ctx, |nav| nav.owner.and_then(|id| nav.folders.get(&id).cloned()))
}

pub(super) fn push_browser_parent(ui: &Ui, id: egui::Id, open: bool) {
    state(ui.ctx(), |nav| nav.parent_stack.push((id, open)));
}

pub(super) fn pop_browser_parent(ui: &Ui) {
    state(ui.ctx(), |nav| { nav.parent_stack.pop(); });
}

pub(super) fn browser_navigation_building(ui: &Ui) -> bool {
    state(ui.ctx(), |nav| nav.collecting)
}

pub(super) fn browser_navigation_target(ui: &Ui) -> Option<(String, egui::Id)> {
    state(ui.ctx(), |nav| (nav.scope == nav.owner_scope).then(|| nav.target.clone()).flatten())
}

pub(super) fn browser_navigation_pending(ctx: &egui::Context) -> bool {
    state(ctx, |nav| nav.target.is_some() || nav.step.is_some())
}

pub(super) fn browser_navigation_arrived(ui: &Ui) {
    state(ui.ctx(), |nav| nav.target = None);
}

pub(super) fn browser_asset_navigation_id(ui: &Ui, key: &str) -> egui::Id {
    state(ui.ctx(), |nav| nav.scope.unwrap_or(ui.id()).with(("asset", key)))
}

pub(super) fn browser_navigation_columns(ui: &Ui, columns: usize, row_height: f32) {
    state(ui.ctx(), |nav| { if nav.collecting {
        nav.columns = columns;
        nav.page_rows = (ui.clip_rect().height() / row_height).floor().max(1.0) as usize;
    } });
}

fn move_selection(nav: &mut Navigation, key: egui::Key) -> Option<NavigationAction> {
    let index = nav.rows.iter().position(|(_, id)| Some(*id) == nav.owner)?;
    let (current_key, current_id) = nav.rows[index].clone();
    let folder = nav.folders.get(&current_id);
    let next = match key {
        egui::Key::Home => 0,
        egui::Key::End => nav.rows.len().saturating_sub(1),
        egui::Key::PageUp | egui::Key::PageDown => {
            let page = nav.page_rows.max(1) * nav.columns.max(1);
            if key == egui::Key::PageUp { index.saturating_sub(page) }
            else { (index + page).min(nav.rows.len().saturating_sub(1)) }
        }
        egui::Key::ArrowRight => {
            let folder = folder?;
            if !folder.open {
                return Some(NavigationAction::Toggle(current_key, current_id, folder.collapse_id?, true));
            }
            let next = index + 1;
            let (_, child_id) = nav.rows.get(next)?;
            if nav.parents.get(child_id).copied().flatten() != Some(current_id) { return None; }
            next
        }
        egui::Key::ArrowLeft => {
            if let Some(folder) = folder.filter(|folder| folder.open)
                && let Some(collapse_id) = folder.collapse_id
            {
                return Some(NavigationAction::Toggle(current_key, current_id, collapse_id, false));
            }
            let parent = nav.parents.get(&current_id).copied().flatten()?;
            nav.rows.iter().position(|(_, id)| *id == parent)?
        }
        _ => {
            let step = if key == egui::Key::ArrowUp { -1 } else { 1 };
            index.saturating_add_signed(step * nav.columns.max(1) as isize).min(nav.rows.len().saturating_sub(1))
        }
    };
    let target = nav.rows.get(next)?.clone();
    nav.owner = Some(target.1);
    nav.target = Some(target.clone());
    Some(NavigationAction::Select(target.0, target.1))
}

fn apply_navigation(ctx: &egui::Context, action: NavigationAction) -> (String, egui::Id) {
    match action {
        NavigationAction::Select(key, id) => (key, id),
        NavigationAction::Toggle(key, id, collapse_id, open) => {
            let mut collapse = egui::collapsing_header::CollapsingState::load_with_default_open(ctx, collapse_id, !open);
            collapse.set_open(open);
            collapse.store(ctx);
            state(ctx, |nav| {
                nav.ready = false;
                nav.target = Some((key.clone(), id));
                if let Some(folder) = nav.folders.get_mut(&id) { folder.open = open; }
            });
            (key, id)
        }
    }
}

/// The first arrow press collects the expanded tree once. Subsequent presses
/// reuse that order until another interaction invalidates browser selection.
pub(super) fn move_browser_selection(ctx: &egui::Context, key: egui::Key) -> Option<(String, egui::Id)> {
    let target = state(ctx, |nav| {
        if nav.ready { move_selection(nav, key) }
        else { nav.step = Some(key); None }
    }).map(|action| apply_navigation(ctx, action));
    ctx.request_repaint();
    target
}

pub(super) fn finish_browser_navigation(ui: &Ui) {
    let target = state(ui.ctx(), |nav| {
        if !nav.collecting { return None; }
        nav.collecting = false;
        nav.ready = true;
        let step = nav.step.take()?;
        move_selection(nav, step)
    });
    if let Some(action) = target {
        let (key, id) = apply_navigation(ui.ctx(), action);
        set_browser_selected_tag(ui, Some(key));
        ui.memory_mut(|memory| memory.request_focus(id));
        ui.data_mut(|data| data.insert_temp(browser_selection_owner_id(), (id, ui.clip_rect())));
        ui.ctx().request_repaint();
    }
}
