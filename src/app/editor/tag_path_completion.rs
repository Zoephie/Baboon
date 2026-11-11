//! The list under a box a tag reference or tag path is typed in: the folders
//! and tags the text could go on to name, from the kit the box belongs to.
//!
//! Built on egui's [`egui::CompletionPopup`], which takes the arrow keys,
//! Enter, Tab and Escape while the list is open and keeps the box focused
//! while a row is clicked, so a click doesn't first commit the half-typed
//! text. Nothing is completed by itself: the text changes only when a row is
//! picked.

use super::*;
use crate::core::tag_path_completion::{
    Catalog, FolderListing, Suggestion, SuggestionKind, Wanted, list_folder, split_typed,
    suggestions,
};

/// The most rows the list holds. A folder can hold thousands of tags; typing
/// more of the name narrows them.
const MOST_ROWS: usize = 200;

/// The list's height before it scrolls.
const LIST_HEIGHT: f32 = 12.0 * 22.0;

/// What a box's list offers, and where from.
#[derive(Clone, Copy)]
pub(in crate::app) struct TagPathCompletion<'a> {
    pub(in crate::app) catalog: Catalog<'a>,
    pub(in crate::app) wanted: Wanted<'a>,
    /// The game whose icons the tags are shown with.
    pub(in crate::app) game: Option<GameId>,
}

/// When a box's list may open: once its text has been changed, so focusing
/// a box doesn't cover what is below it.
pub(in crate::app) struct Offer {
    /// The draft held a change before this frame.
    changed: bool,
    /// The text before this frame's typing, which opens the list on the
    /// frame the typing lands rather than on the next one drawn.
    before: String,
}

impl Offer {
    pub(in crate::app) fn new(changed: bool, before: &str) -> Self {
        Self { changed, before: before.to_owned() }
    }

    fn holds(&self, text: &str) -> bool {
        self.changed || text != self.before
    }
}

/// A row picked from the list. The box already holds its text: a folder's
/// path and a separator, or a tag's path and extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum Picked {
    /// Typing goes on inside the folder.
    Folder,
    Tag { group_tag: u32, path: String },
}

impl<'a> TagPathCompletion<'a> {
    /// What a field's or dialog's kit offers: the container catalog when the
    /// kit has one, otherwise its loose tags folder; `None` without either.
    pub(in crate::app) fn for_kit(
        entries: Option<&'a [TagEntry]>,
        tags_root: Option<&'a Path>,
        names: Option<&'a TagNameIndex>,
        wanted: Wanted<'a>,
        game: Option<GameId>,
    ) -> Option<Self> {
        let catalog = match (entries, tags_root) {
            (Some(entries), _) => Catalog::Entries(entries),
            (None, Some(root)) => Catalog::Loose { root, names },
            (None, None) => return None,
        };
        Some(Self { catalog, wanted, game })
    }

    /// The rows for `typed`, best first; none once it already names one of
    /// the tags it would list.
    pub(in crate::app) fn rows(&self, ctx: &egui::Context, typed: &str) -> Vec<Suggestion> {
        let Some((folder, partial)) = split_typed(typed) else {
            return Vec::new();
        };
        let listing = self.listing(ctx, &folder);
        let mut rows = suggestions(&listing, &folder, &partial, self.wanted);
        let typed = tag_path::from_input(typed.trim());
        if rows
            .iter()
            .any(|row| row.extension.is_some() && row.text().eq_ignore_ascii_case(&typed))
        {
            return Vec::new();
        }
        rows.truncate(MOST_ROWS);
        rows
    }

    /// One folder's listing, read again at most once a second: the list is
    /// drawn every frame it is open.
    fn listing(&self, ctx: &egui::Context, folder: &str) -> FolderListing {
        let catalog = self.catalog;
        match catalog {
            Catalog::Loose { root, .. } => crate::app::shell::frame::recheck_cached(
                ctx,
                ("tag_path_completion", root, folder),
                || list_folder(catalog, folder),
            ),
            Catalog::Entries(entries) => crate::app::shell::frame::recheck_cached(
                ctx,
                ("tag_path_completion", entries.as_ptr() as usize, entries.len(), folder),
                || list_folder(catalog, folder),
            ),
        }
    }

    /// Show `edit` (given the id `id`) with the list under it, open only when
    /// `offer` holds.
    pub(in crate::app) fn show(
        &self,
        ui: &mut Ui,
        id: egui::Id,
        edit: egui::TextEdit<'_>,
        offer: &Offer,
    ) -> (egui::text_edit::TextEditOutput, Option<Picked>) {
        let ctx = ui.ctx().clone();
        let folder_icon = button_icon_image(ui, ButtonIcon::FolderClosed, text_dark(), 16.0);
        let mut rows = Vec::new();
        let output = egui::CompletionPopup::new(id)
            // The whole text before the cursor is the path being completed.
            .word_boundary(|_| false)
            .align(egui::RectAlign::BOTTOM_START)
            .max_height(LIST_HEIGHT)
            .show(ui, edit, |query| {
                if !offer.holds(query.text) {
                    return Vec::new();
                }
                rows = self.rows(&ctx, query.word);
                rows.iter()
                    .map(|row| {
                        let icon = match row.kind {
                            SuggestionKind::Folder => folder_icon.clone(),
                            SuggestionKind::Tag { group_tag } => {
                                tag_icon_image(&ctx, tag_icon(Some(group_tag), self.game), 16.0)
                                    .fit_to_exact_size(Vec2::splat(16.0))
                            }
                        };
                        let suggestion = egui::Suggestion::new(row.text())
                            .content((icon, RichText::new(&row.label).monospace()));
                        match row.kind {
                            SuggestionKind::Folder => suggestion.description("folder"),
                            SuggestionKind::Tag { .. } => suggestion,
                        }
                    })
                    .collect()
            });
        ctx.data_mut(|data| data.insert_temp(open_id(id), output.is_open));
        let picked = output.accepted.and_then(|accepted| {
            let row = rows.iter().find(|row| row.text() == accepted.insert)?;
            Some(match row.kind {
                SuggestionKind::Folder => Picked::Folder,
                SuggestionKind::Tag { group_tag } => Picked::Tag {
                    group_tag,
                    path: row.path.clone(),
                },
            })
        });
        (output.text_edit, picked)
    }
}

/// Whether the list under the box `id` was open on the last frame. Escape
/// closes an open list and does nothing else, so a box that also handles
/// Escape itself asks this first.
pub(in crate::app) fn completion_list_open(ui: &Ui, id: egui::Id) -> bool {
    ui.data(|data| data.get_temp::<bool>(open_id(id))).unwrap_or(false)
}

fn open_id(id: egui::Id) -> egui::Id {
    id.with("tag path completion open")
}
