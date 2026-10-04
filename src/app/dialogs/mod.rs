//! The dialog host: the windows the application opens over its workspaces, owned and drawn in one place.
//! It owns which dialogs are open and drawing them; what a dialog shows and what it asks for belong to the feature that defines it.

use super::*;
use crate::app::kits::KitsFeature;
use crate::app::search::SearchFeature;
use std::any::{Any, TypeId};

/// A window the [`DialogHost`] owns and draws.
///
/// A dialog holds its own state — its draft, its choices, what it was opened
/// on — and reaches the rest of the application only through what it is drawn
/// with: the [`Ctx`], to read the model and send what it decides as commands,
/// and [`AppReads`], to read other features' state. Anything that has to
/// change an open dialog afterwards, such as a handler reporting why a
/// confirmed action was refused, finds it in the host by type.
pub(in crate::app) trait Dialog: Any {
    /// Draw the dialog for this frame. Returns whether it stays open.
    fn show(&mut self, cx: &Ctx, app: &AppReads) -> bool;

    /// Tells apart open dialogs of one type. Opening a dialog whose type and
    /// instance match an open one replaces it, so by default a type is open
    /// at most once.
    fn instance(&self) -> u64 {
        0
    }
}

/// Other features' state a dialog may read while it draws, lent by the
/// application for the frame. Read-only: a dialog changes only its own state,
/// and asks for anything else with a command.
pub(in crate::app) struct AppReads<'a> {
    pub(in crate::app) kit_tools: &'a KitsFeature,
    pub(in crate::app) search: &'a SearchFeature,
    pub(in crate::app) shell: &'a ShellFeature,
}

/// The [`AppReads`] of an application, borrowed beside its dialogs.
macro_rules! app_reads {
    ($app:expr) => {
        $crate::app::dialogs::AppReads {
            kit_tools: &$app.kit_tools,
            search: &$app.search,
            shell: &$app.shell,
        }
    };
}
pub(in crate::app) use app_reads;

/// Every open dialog, in the order they were opened.
#[derive(Default)]
pub(in crate::app) struct DialogHost {
    open: Vec<Box<dyn Dialog>>,
}

/// What makes an open dialog the same as another: its type and instance.
fn key_of(dialog: &dyn Dialog) -> (TypeId, u64) {
    ((dialog as &dyn Any).type_id(), dialog.instance())
}

impl DialogHost {
    /// Open `dialog`, replacing an open one with the same type and instance
    /// where it stands.
    pub(in crate::app) fn open(&mut self, dialog: impl Dialog) {
        self.open_boxed(Box::new(dialog));
    }

    pub(in crate::app) fn open_boxed(&mut self, dialog: Box<dyn Dialog>) {
        let key = key_of(&*dialog);
        match self.open.iter_mut().find(|open| key_of(&***open) == key) {
            Some(slot) => *slot = dialog,
            None => self.open.push(dialog),
        }
    }

    /// The first open dialog of type `T`.
    pub(in crate::app) fn get<T: Dialog>(&self) -> Option<&T> {
        self.open
            .iter()
            .find_map(|dialog| (&**dialog as &dyn Any).downcast_ref::<T>())
    }

    pub(in crate::app) fn get_mut<T: Dialog>(&mut self) -> Option<&mut T> {
        self.open
            .iter_mut()
            .find_map(|dialog| (&mut **dialog as &mut dyn Any).downcast_mut::<T>())
    }

    /// Close the first open dialog of type `T`, handing it back.
    pub(in crate::app) fn close<T: Dialog>(&mut self) -> Option<T> {
        let index = self
            .open
            .iter()
            .position(|dialog| (&**dialog as &dyn Any).is::<T>())?;
        let dialog: Box<dyn Any> = self.open.remove(index);
        dialog.downcast::<T>().ok().map(|dialog| *dialog)
    }

    /// Draw every open dialog, dropping the ones that closed.
    pub(in crate::app) fn draw(&mut self, cx: &Ctx, app: &AppReads) {
        self.open.retain_mut(|dialog| dialog.show(cx, app));
    }
}

#[cfg(test)]
mod tests;
