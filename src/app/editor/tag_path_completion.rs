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
    /// A picked tag's text carries its extension. Tool command arguments
    /// name a tag without one, as their Browse button gives it.
    pub(in crate::app) extensions: bool,
}

/// When a box's list may open: once its text has been changed, so focusing
/// a box doesn't cover what is below it. Once offered, the list stays on
/// offer until the box loses focus.
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
/// path and a separator, or a tag's path (and extension, unless the box takes
/// tags without one).
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
        Some(Self { catalog, wanted, game, extensions: true })
    }

    /// What the kit at `kit` in `model` offers, for a dialog raised on it.
    pub(in crate::app) fn for_model_kit(model: &'a Model, kit: usize, wanted: Wanted<'a>) -> Option<Self> {
        let kit = model.kits.get(kit)?;
        let source = kit.source.as_ref()?;
        let catalog = match &source.source {
            TagSource::LooseFolder { root, .. } => Catalog::Loose { root, names: Some(&kit.names) },
            TagSource::IoStoreContainerSet { .. } => Catalog::Entries(&source.entries),
            _ => return None,
        };
        Some(Self { catalog, wanted, game: source.game, extensions: true })
    }

    /// Picked tags go in without their extension.
    pub(in crate::app) fn without_extensions(self) -> Self {
        Self { extensions: false, ..self }
    }

    /// What picking `row` puts in the box.
    fn insert(&self, row: &Suggestion) -> String {
        match row.kind {
            SuggestionKind::Tag { .. } if !self.extensions => row.path.clone(),
            _ => row.text(),
        }
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
        if rows.iter().any(|row| {
            matches!(row.kind, SuggestionKind::Tag { .. }) && self.insert(row).eq_ignore_ascii_case(&typed)
        }) {
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
        let offered_before = ctx.data(|data| data.get_temp::<bool>(offered_id(id))).unwrap_or(false);
        let mut offered = false;
        let mut rows = Vec::new();
        let output = egui::CompletionPopup::new(id)
            // The whole text before the cursor is the path being completed.
            .word_boundary(|_| false)
            .align(egui::RectAlign::BOTTOM_START)
            .max_height(LIST_HEIGHT)
            .show(ui, edit, |query| {
                offered = offered_before || offer.holds(query.text);
                if !offered {
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
                        let suggestion = egui::Suggestion::new(self.insert(row))
                            .content((icon, RichText::new(&row.label).monospace()));
                        match row.kind {
                            SuggestionKind::Folder => suggestion.description("folder"),
                            SuggestionKind::Tag { .. } => suggestion,
                        }
                    })
                    .collect()
            });
        let focused = output.text_edit.response.has_focus();
        ctx.data_mut(|data| {
            data.insert_temp(open_id(id), output.is_open);
            data.insert_temp(offered_id(id), offered && focused);
        });
        let picked = output.accepted.and_then(|accepted| {
            let row = rows.iter().find(|row| self.insert(row) == accepted.insert)?;
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

fn offered_id(id: egui::Id) -> egui::Id {
    id.with("tag path completion offered")
}

/// Add a dialog's path box `edit`, given the id `id`, with `completion`'s
/// list under it once its text is changed from `before` (what it held at the
/// start of the frame). Returns the response, and whether the text changed
/// by typing or by a pick: a picked row's text is in the box already.
pub(in crate::app) fn add_completing(
    ui: &mut Ui,
    edit: crate::app::ui_kit::SinglelineTextEdit<'_>,
    id: egui::Id,
    completion: Option<&TagPathCompletion<'_>>,
    before: &str,
) -> (egui::Response, bool) {
    let edit = edit.id(id);
    let Some(completion) = completion else {
        let response = ui.add(edit);
        let changed = response.changed();
        return (response, changed);
    };
    let offer = Offer::new(false, before);
    let mut picked = None;
    let response = edit
        .show_through(ui, &mut |ui, text_edit| {
            let (output, row) = completion.show(ui, id, text_edit, &offer);
            picked = row;
            output
        })
        .response
        .response;
    let changed = response.changed() || picked.is_some();
    (response, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::LooseKit;

    /// A dialog's path box over a Halo 3 kit holding a weapon, a crate and
    /// two folders under `objects`.
    struct PathBox {
        fixture: LooseKit,
        names: TagNameIndex,
        ctx: egui::Context,
        text: String,
        time: f64,
        /// What the box takes, and whether a picked tag goes in bare.
        wanted: Wanted<'static>,
        bare: bool,
    }

    const ID: &str = "test_path_box";

    impl PathBox {
        fn new(name: &str) -> Self {
            let fixture = LooseKit::new(name, "halo3_mcc");
            for file in [r"objects\weapons\rifle\rifle.weapon", r"objects\scenery\rock\rock.scenery", r"objects\crate.scenery"] {
                let path = fixture.path(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, []).unwrap();
            }
            let names = fixture.names();
            let ctx = egui::Context::default();
            egui_extras::install_image_loaders(&ctx);
            Self { fixture, names, ctx, text: String::new(), time: 0.0, wanted: Wanted::Folders, bare: false }
        }

        fn taking(mut self, wanted: Wanted<'static>, bare: bool) -> Self {
            self.wanted = wanted;
            self.bare = bare;
            self
        }

        /// Draw the box once; the output and whether its text changed.
        fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, bool) {
            self.time += 1.0 / 60.0;
            let mut changed = false;
            let mut completion = TagPathCompletion {
                catalog: Catalog::Loose { root: &self.fixture.root, names: Some(&self.names) },
                wanted: self.wanted,
                game: Some(GameId::Halo3),
                extensions: true,
            };
            if self.bare {
                completion = completion.without_extensions();
            }
            let text = &mut self.text;
            let output = crate::app::run_ui_test(&self.ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 500.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let before = text.clone();
                    let edit = crate::app::ui_kit::singleline_text_edit(text).desired_width(400.0);
                    changed = add_completing(ui, edit, egui::Id::unique(ID), Some(&completion), &before).1;
                });
            });
            (output, changed)
        }

        /// Focus the box and type `typed` into it, as two frames: the list
        /// paints nothing on the frame it opens.
        fn type_text(&mut self, typed: &str) -> egui::FullOutput {
            self.ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique(ID)));
            self.frame(Vec::new());
            self.frame(vec![egui::Event::Text(typed.to_owned())]);
            self.frame(Vec::new()).0
        }

        fn enter(&mut self) -> bool {
            self.frame(vec![egui::Event::Key {
                key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]).1
        }
    }

    fn shows(output: &egui::FullOutput, text: &str) -> bool {
        output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(shown) if shown.galley.text() == text))
    }

    /// A folder box lists folders only, and picking one puts it in the box
    /// to go on typing, as a change the dialog sees.
    #[test]
    fn a_folder_box_lists_folders_and_takes_a_picked_one() {
        let mut path_box = PathBox::new("completion-folders").taking(Wanted::Folders, false);
        let output = path_box.type_text(r"objects\");
        assert!(shows(&output, "scenery") && shows(&output, "weapons"));
        assert!(!shows(&output, "crate.scenery"), "a folder box listed a tag");
        assert!(path_box.enter(), "the pick is a change");
        assert_eq!(path_box.text, r"objects\scenery\");
    }

    /// With no draft to say the text has changed, the box remembers it was
    /// offered until it loses focus, and focusing it again doesn't reopen
    /// the list.
    #[test]
    fn a_dialog_box_keeps_its_list_until_it_loses_focus() {
        let mut path_box = PathBox::new("completion-offer").taking(Wanted::Folders, false);
        path_box.type_text(r"objects\");
        let (output, _) = path_box.frame(Vec::new());
        assert!(shows(&output, "weapons"), "the list closed while nothing was typed");
        path_box.ctx.memory_mut(|memory| memory.surrender_focus(egui::Id::unique(ID)));
        let (output, _) = path_box.frame(Vec::new());
        assert!(!shows(&output, "weapons"));
        path_box.ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique(ID)));
        path_box.frame(Vec::new());
        let (output, _) = path_box.frame(Vec::new());
        assert!(!shows(&output, "weapons"), "focusing the box again reopened the list");
    }

    /// A box taking tags without an extension puts a picked tag's bare path
    /// in, and counts that path, typed out, as finished.
    #[test]
    fn a_box_without_extensions_takes_the_bare_path() {
        let mut path_box = PathBox::new("completion-bare").taking(Wanted::Tags(None), true);
        let output = path_box.type_text(r"objects\cra");
        assert!(shows(&output, "crate.scenery"));
        assert!(path_box.enter());
        assert_eq!(path_box.text, r"objects\crate");
        let (output, _) = path_box.frame(Vec::new());
        path_box.frame(Vec::new());
        assert!(!shows(&output, "crate.scenery"), "the finished path still lists itself");

        let mut path_box = PathBox::new("completion-extension").taking(Wanted::Tags(None), false);
        path_box.type_text(r"objects\cra");
        path_box.enter();
        assert_eq!(path_box.text, r"objects\crate.scenery");
    }

    /// A dialog's list comes from the kit it was raised on: a loose kit's
    /// folder with its own names and game, and nothing for an empty
    /// workspace.
    #[test]
    fn a_dialog_lists_its_own_kit() {
        let fixture = LooseKit::new("completion-model-kit", "halo3_mcc");
        let mut app = Baboon::for_test();
        assert!(TagPathCompletion::for_model_kit(&app.model, 0, Wanted::Folders).is_none());
        fixture.install(&mut app);
        let completion = TagPathCompletion::for_model_kit(&app.model, 0, Wanted::Folders).unwrap();
        assert!(matches!(completion.catalog, Catalog::Loose { root, names: Some(_) } if root == fixture.root));
        assert_eq!(completion.game, Some(GameId::Halo3));
        assert!(TagPathCompletion::for_model_kit(&app.model, 5, Wanted::Folders).is_none());
    }
}
