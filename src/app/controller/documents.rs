//! Document and tab-selection helpers.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::TagLoaded`, discarding results for tabs closed while loading.
    pub(super) fn handle_tag_loaded(
        &mut self,
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    ) -> bool {
        // Routed by kit id rather than generation: a parsed document stays
        // valid across a source reload, and must land in the kit that asked
        // for it even if the user has since switched to another.
        let Some(index) = self.resolve_kit(kit) else {
            return true;
        };
        self.kits[index].loading_tags.remove(&key);
        if !self.kits[index].open_tabs.iter().any(|tab| tab == &key) {
            return true;
        }
        match result {
            Ok(tag) => {
                self.status = "Tag loaded".to_owned();
                self.kits[index]
                    .parsed_tags
                    .insert(key.clone(), TagDocument::clean(tag));
                // A restored session stages this tag's undo history before the
                // read that produces the document, so it is waiting here.
                self.apply_pending_history(index, &key);
            }
            Err(error) => {
                self.terminal.lines.push(TerminalLineEntry::new(format!(
                    "Folder refactor failed: {error}"
                )));
                trim_terminal_lines(&mut self.terminal.lines);
                self.terminal.scroll_to_bottom = true;
                self.status = error;
            }
        }
        false
    }

    /// Applies `WorkerMessage::BitmapReimportFinished` and reloads an open bitmap document.
    pub(super) fn handle_bitmap_reimport_finished(
        &mut self,
        kit: KitId,
        key: String,
        result: Result<TagFile, String>,
    ) -> bool {
        // The terminal reset is global and must run even when the owning kit
        // has closed, so it happens before the routing check.
        self.terminal.running = false;
        self.terminal.running_id = None;
        self.terminal.running_command = None;
        self.terminal.process = None;
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
        let Some(index) = self.resolve_kit(kit) else {
            return true;
        };
        match result {
            Ok(tag) => {
                if self.kits[index].open_tabs.iter().any(|tab| tab == &key) {
                    self.kits[index]
                        .parsed_tags
                        .insert(key.clone(), TagDocument::clean(tag));
                    self.kits[index].bitmap_previews.remove(&key);
                }
                self.status = "Bitmap reimported and reloaded".to_owned();
            }
            Err(error) => self.status = format!("Bitmap reimport failed: {error}"),
        }
        false
    }
}
