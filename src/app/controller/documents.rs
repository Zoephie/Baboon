//! Document and tab-selection helpers.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::TagLoaded`, discarding results for tabs closed while loading.
    pub(super) fn handle_tag_loaded(
        &mut self,
        key: String,
        result: Result<TagFile, String>,
    ) -> bool {
        self.loading_tags.remove(&key);
        if !self.open_tabs.iter().any(|tab| tab == &key) {
            return true;
        }
        match result {
            Ok(tag) => {
                self.status = "Tag loaded".to_owned();
                self.remember_tag_use(&key);
                self.parsed_tags.insert(key, TagDocument::clean(tag));
                self.trim_tag_memory();
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
        key: String,
        result: Result<TagFile, String>,
    ) -> bool {
        self.terminal.running = false;
        self.terminal.running_id = None;
        self.terminal.running_command = None;
        self.terminal.process = None;
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
        match result {
            Ok(tag) => {
                if self.open_tabs.iter().any(|tab| tab == &key) {
                    self.parsed_tags
                        .insert(key.clone(), TagDocument::clean(tag));
                    self.bitmap_previews.remove(&key);
                    self.remember_tag_use(&key);
                    self.trim_tag_memory();
                }
                self.status = "Bitmap reimported and reloaded".to_owned();
            }
            Err(error) => self.status = format!("Bitmap reimport failed: {error}"),
        }
        false
    }
}

pub(super) fn selected_tab_after_removal(
    open_tabs: &[String],
    removed_index: Option<usize>,
) -> Option<String> {
    let removed_index = removed_index?;
    if open_tabs.is_empty() {
        None
    } else {
        open_tabs
            .get(removed_index)
            .or_else(|| {
                removed_index
                    .checked_sub(1)
                    .and_then(|index| open_tabs.get(index))
            })
            .cloned()
    }
}
