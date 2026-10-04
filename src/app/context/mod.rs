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

/// A draw's view of the application. Built from Baboon's fields with
/// [`Ctx::new`] rather than from `&Baboon`, so the draw can hold its own
/// feature's state mutably beside it.
pub(in crate::app) struct Ctx<'a> {
    pub(in crate::app) model: &'a Model,
    pub(in crate::app) egui: &'a egui::Context,
    commands: &'a CommandQueue,
}

impl<'a> Ctx<'a> {
    pub(in crate::app) fn new(model: &'a Model, egui: &'a egui::Context, commands: &'a CommandQueue) -> Self {
        Self { model, egui, commands }
    }

    /// Queue `command` to run once this frame's drawing is over.
    pub(in crate::app) fn send(&self, command: impl Into<Command>) {
        self.commands.send(command);
    }

    /// Replace the status line once this frame's drawing is over.
    pub(in crate::app) fn set_status(&self, status: impl Into<String>) {
        self.commands.send(Command::Status(status.into()));
    }
}

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

    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut *self.queued.borrow_mut())
    }
}

/// Everything a draw can ask for: one variant per feature, each wrapping that
/// feature's own command type.
pub(in crate::app) enum Command {
    /// Replace the status line.
    Status(String),
    Help(HelpCommand),
    Poke(PokeCommand),
}

impl From<HelpCommand> for Command {
    fn from(command: HelpCommand) -> Self {
        Command::Help(command)
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
                self.apply_command(command);
            }
        }
    }

    fn apply_command(&mut self, command: Command) {
        match command {
            Command::Status(status) => self.model.status = status,
            Command::Help(command) => self.apply_help_command(command),
            Command::Poke(command) => self.apply_poke_command(command),
        }
    }
}

#[cfg(test)]
mod tests;
