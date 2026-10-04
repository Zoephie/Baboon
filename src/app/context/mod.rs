//! What a draw is given, and how it asks for change.
//!
//! A feature's UI draws from a [`Ctx`] (the [`Model`] read-only, the egui
//! context, and a [`CommandQueue`]) plus its own feature state held mutably.
//! What it wants done to anything else it sends as a [`Command`]; the frame
//! applies every queued command once drawing is over, in the order sent, in
//! [`Baboon::apply_commands`]. So a draw cannot change the model under a
//! sibling drawn after it, and what a click does is one value a test can
//! read without a window.

use std::cell::RefCell;

use super::*;
use crate::app::shell::{AppAction, FirstRunCommand, SettingsCommand, WorkerMessage, spawn_worker};
use crate::app::documents::DocumentsCommand;
use crate::app::kits::KitsCommand;
use crate::app::tag_ops::TagOpsCommand;
use crate::app::mods::ModsCommand;
use crate::app::import::ImportCommand;
use crate::app::references::ReferencesCommand;
use crate::app::search::SearchCommand;
use crate::app::compare::CompareCommand;
use crate::app::help::HelpCommand;
use crate::app::chimp::ChimpCommand;
use crate::app::runtime_poke::PokeCommand;
use crate::app::editor::EditorCommand;
use crate::app::export::ExportCommand;
use crate::app::browser::BrowserCommand;

/// A draw's view of the application. Built from Baboon's fields with
/// [`Ctx::new`] rather than from `&Baboon`, so the draw can hold its own
/// feature's state mutably beside it.
pub(in crate::app) struct Ctx<'a> {
    pub(in crate::app) model: &'a Model,
    pub(in crate::app) egui: &'a egui::Context,
    commands: &'a CommandQueue,
    jobs: &'a Sender<WorkerMessage>,
}

impl<'a> Ctx<'a> {
    pub(in crate::app) fn new(
        model: &'a Model,
        egui: &'a egui::Context,
        commands: &'a CommandQueue,
        jobs: &'a Sender<WorkerMessage>,
    ) -> Self {
        Self {
            model,
            egui,
            commands,
            jobs,
        }
    }

    /// Queue `command` to run once this frame's drawing is over.
    pub(in crate::app) fn send(&self, command: impl Into<Command>) {
        self.commands.send(command);
    }

    /// Open `dialog` once this frame's drawing is over.
    pub(in crate::app) fn open_dialog(&self, dialog: impl Dialog) {
        self.commands.send(Command::OpenDialog(Box::new(dialog)));
    }

    /// Replace the status line once this frame's drawing is over.
    pub(in crate::app) fn set_status(&self, status: impl Into<String>) {
        self.commands.send(Command::Status(status.into()));
    }

    /// Change the live preferences with `edit` once this frame's drawing is
    /// over. A change rather than a whole new set, so two draws changing
    /// different preferences in one frame both land.
    pub(in crate::app) fn edit_prefs(&self, edit: impl FnOnce(&mut GuiPrefs) + 'static) {
        self.commands.send(Command::EditPrefs(Box::new(edit)));
    }

    /// Show `path` in the platform's file manager once this frame's drawing
    /// is over.
    pub(in crate::app) fn open_folder(&self, path: PathBuf, label: &str) {
        self.commands.send(Command::OpenFolder {
            path,
            label: label.to_owned(),
        });
    }

    /// Run `job` in the background; its message comes back to the frame
    /// that receives worker messages, and `on_panic` stands in for it if the
    /// job panics. Starting work changes nothing a draw reads, so a draw may
    /// do it directly; what the result changes is up to its handler.
    pub(in crate::app) fn spawn<J, P>(&self, job: J, on_panic: P)
    where
        J: FnOnce() -> WorkerMessage + Send + 'static,
        P: FnOnce(String) -> WorkerMessage + Send + 'static,
    {
        spawn_worker(self.jobs, self.egui, job, on_panic);
    }
}

/// The [`Ctx`] for this frame, built from `app`'s fields so the caller can
/// still hold a feature's state mutably beside it. A macro rather than a
/// method because a method would borrow all of `app`.
macro_rules! cx {
    ($app:expr, $egui:expr) => {
        $crate::app::context::Ctx::new(&$app.model, $egui, &$app.commands, &$app.tx)
    };
}
pub(in crate::app) use cx;

/// Commands sent this frame and not yet applied. Shared rather than `&mut`
/// so the [`Ctx`] holding it can be shared too: a draw passes one `&Ctx` to
/// every helper it calls, beside the `&mut` state of its own feature.
#[derive(Default)]
pub(in crate::app) struct CommandQueue {
    queued: RefCell<Vec<Command>>,
}

impl CommandQueue {
    /// Queue `command`. Also how code not yet drawing from a [`Ctx`] asks
    /// for a migrated feature's change, so it lands at the same point in the
    /// frame as the same request sent from a draw.
    pub(in crate::app) fn send(&self, command: impl Into<Command>) {
        self.queued.borrow_mut().push(command.into());
    }

    /// How many commands are waiting, for tests whose command cannot be
    /// applied without a person (one that opens a file picker).
    #[cfg(test)]
    pub(in crate::app) fn len(&self) -> usize {
        self.queued.borrow().len()
    }

    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut *self.queued.borrow_mut())
    }
}

/// Everything a draw can ask for: one variant per feature, each wrapping that
/// feature's own command type.
pub(in crate::app) enum Command {
    /// Replace the status line.
    Status(String),
    /// Show a folder in the platform's file manager; `label` names it in the
    /// status line if that fails.
    OpenFolder { path: PathBuf, label: String },
    /// Change the live preferences.
    EditPrefs(Box<dyn FnOnce(&mut GuiPrefs)>),
    Help(HelpCommand),
    Poke(PokeCommand),
    Compare(CompareCommand),
    Search(SearchCommand),
    References(ReferencesCommand),
    Export(ExportCommand),
    Mods(ModsCommand),
    TagOps(TagOpsCommand),
    Import(ImportCommand),
    Documents(DocumentsCommand),
    Editor(EditorCommand),
    Browser(BrowserCommand),
    Kits(KitsCommand),
    Audio(AudioCommand),
    App(AppAction),
    Settings(SettingsCommand),
    FirstRun(FirstRunCommand),
    Chimp(ChimpCommand),
    /// Open a dialog, replacing an open one of its type and instance.
    OpenDialog(Box<dyn Dialog>),
}

impl From<HelpCommand> for Command {
    fn from(command: HelpCommand) -> Self {
        Command::Help(command)
    }
}

impl From<ChimpCommand> for Command {
    fn from(command: ChimpCommand) -> Self {
        Command::Chimp(command)
    }
}

impl From<CompareCommand> for Command {
    fn from(command: CompareCommand) -> Self {
        Command::Compare(command)
    }
}

impl From<SearchCommand> for Command {
    fn from(command: SearchCommand) -> Self {
        Command::Search(command)
    }
}

impl From<ReferencesCommand> for Command {
    fn from(command: ReferencesCommand) -> Self {
        Command::References(command)
    }
}

impl From<ExportCommand> for Command {
    fn from(command: ExportCommand) -> Self {
        Command::Export(command)
    }
}

impl From<ModsCommand> for Command {
    fn from(command: ModsCommand) -> Self {
        Command::Mods(command)
    }
}

impl From<TagOpsCommand> for Command {
    fn from(command: TagOpsCommand) -> Self {
        Command::TagOps(command)
    }
}

impl From<ImportCommand> for Command {
    fn from(command: ImportCommand) -> Self {
        Command::Import(command)
    }
}

impl From<DocumentsCommand> for Command {
    fn from(command: DocumentsCommand) -> Self {
        Command::Documents(command)
    }
}

impl From<EditorCommand> for Command {
    fn from(command: EditorCommand) -> Self {
        Command::Editor(command)
    }
}

impl From<BrowserCommand> for Command {
    fn from(command: BrowserCommand) -> Self {
        Command::Browser(command)
    }
}

impl From<KitsCommand> for Command {
    fn from(command: KitsCommand) -> Self {
        Command::Kits(command)
    }
}

impl From<AudioCommand> for Command {
    fn from(command: AudioCommand) -> Self {
        Command::Audio(command)
    }
}

impl From<AppAction> for Command {
    fn from(action: AppAction) -> Self {
        Command::App(action)
    }
}

impl From<SettingsCommand> for Command {
    fn from(command: SettingsCommand) -> Self {
        Command::Settings(command)
    }
}

impl From<FirstRunCommand> for Command {
    fn from(command: FirstRunCommand) -> Self {
        Command::FirstRun(command)
    }
}

impl From<PokeCommand> for Command {
    fn from(command: PokeCommand) -> Self {
        Command::Poke(command)
    }
}

impl Baboon {
    /// Apply every command queued this frame, in the order sent. A command
    /// may queue more; those run in the same pass rather than a frame late.
    /// Drawing has already happened by now, so anything applied repaints.
    pub(in crate::app) fn apply_commands(&mut self, ctx: &egui::Context) {
        loop {
            let commands = self.commands.take();
            if commands.is_empty() {
                return;
            }
            ctx.request_repaint();
            for command in commands {
                self.apply_command(command, ctx);
            }
        }
    }

    fn apply_command(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::Status(status) => self.model.status = status,
            Command::OpenFolder { path, label } => self.open_folder_in_explorer(path, &label),
            Command::EditPrefs(edit) => edit(&mut self.model.prefs),
            Command::Help(command) => self.apply_help_command(command),
            Command::Poke(command) => self.apply_poke_command(command, ctx),
            Command::Compare(command) => self.apply_compare_command(command, ctx),
            Command::Search(command) => self.apply_search_command(command, ctx),
            Command::References(command) => self.apply_references_command(command, ctx),
            Command::Export(command) => self.apply_export_command(command, ctx),
            Command::Mods(command) => self.apply_mods_command(command, ctx),
            Command::TagOps(command) => self.apply_tag_ops_command(command, ctx),
            Command::Import(command) => self.apply_import_command(command, ctx),
            Command::Documents(command) => self.apply_documents_command(command, ctx),
            Command::Editor(command) => self.apply_editor_command(command, ctx),
            Command::Browser(command) => self.apply_browser_command(command, ctx),
            Command::Kits(command) => self.apply_kits_command(command, ctx),
            Command::Audio(command) => self.apply_audio_command(command),
            Command::App(action) => self.apply_app_action(action, ctx),
            Command::Settings(command) => self.apply_settings_command(command, ctx),
            Command::FirstRun(command) => self.apply_first_run_command(command),
            Command::Chimp(command) => self.apply_chimp_command(command, ctx),
            Command::OpenDialog(dialog) => self.dialogs.open_boxed(dialog),
        }
    }
}

#[cfg(test)]
mod tests;
