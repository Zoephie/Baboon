//! The Export Mod dialog's state: its rows, their changes and diffs, and where
//! mods go.

use super::*;

/// A parsed imported tag awaiting a "discard unsaved edits?" confirmation before
/// it overwrites an already-open, dirty document at `target_key`.
/// Pending "throw away everything this workspace has not written into the
/// game" confirmation, listing what it is about to drop.
/// One tag the export is about to write, as reviewed before writing.
pub(in crate::app) struct ModExportRow {
    pub(in crate::app) identity: String,
    pub(in crate::app) display_path: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) kind: ModExportChange,
    pub(in crate::app) include: bool,
    pub(in crate::app) bytes: usize,
    /// Why this tag cannot be exported, when it cannot.
    pub(in crate::app) reason: Option<String>,
    /// The mounted mod currently serving this tag, if one is. The comparison
    /// below the row is against the game's own pack either way, so without this
    /// there is nothing to explain why the editor is showing the mod's values.
    pub(in crate::app) overridden_by: Option<String>,
}

impl ModExportDialog {
    /// The container this export would write, which may be one this workspace has
    /// mounted — that is what makes a re-export of an installed mod impossible
    /// while it is open.
    pub(in crate::app) fn output_utoc(&self) -> PathBuf {
        self.destination().join(format!("{}.utoc", self.stem()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ModExportChange {
    /// A tag this workspace created, with no counterpart in the game.
    New,
    /// An edit to a tag the game ships.
    Modified,
    /// In the workspace's stash, but byte-identical to what the game ships.
    ///
    /// Reaching this state does not require the user to have undone anything
    /// deliberately: a value nudged and put back, or an edit that re-encodes to
    /// the same bytes, leaves the document flagged as modified with nothing to
    /// show for it. Listing that as an unexported change is how a review ends up
    /// asserting the user made a change they did not.
    Unchanged,
    /// In the workspace's project, but no longer resolvable in this source.
    Unresolved,
}

/// A modified tag's field-level differences, computed when its row is first
/// expanded and kept for as long as the review is open.
pub(in crate::app) struct ModRowDiff {
    pub(in crate::app) rows: Vec<TagFieldDiff>,
    /// The two tags the rows were computed from, kept so each change can be
    /// rendered through the real field editor rather than described.
    pub(in crate::app) base: Option<blam_tags::TagFile>,
    pub(in crate::app) edited: Option<blam_tags::TagFile>,
    pub(in crate::app) truncated: bool,
    pub(in crate::app) error: Option<String>,
    /// The rows arranged for display, built on first draw.
    pub(in crate::app) view: std::sync::OnceLock<crate::app::mods::mod_export_window::DiffNode>,
}

/// Review of what Export Mod is about to write, shown before anything is
/// written and before a destination is chosen.
///
/// Holds the captured snapshot rather than re-deriving one on confirm, so what
/// was reviewed and what is written cannot disagree.
pub(in crate::app) struct ModExportDialog {
    pub(in crate::app) kit: KitId,
    /// Opened to look rather than to export: the same review without a
    /// destination or an Export button.
    pub(in crate::app) review_only: bool,
    pub(in crate::app) snapshot: CampaignProjectSnapshot,
    pub(in crate::app) rows: Vec<ModExportRow>,
    pub(in crate::app) name: String,
    pub(in crate::app) folder: PathBuf,
    /// True once the user has accepted overwriting the files already there.
    pub(in crate::app) overwrite_acknowledged: bool,
    /// Rows the user has opened. Diffs are computed on first expansion rather
    /// than up front: each one costs a container read and two parses, which
    /// would make opening the review scale with how much is stashed.
    pub(in crate::app) expanded: HashSet<String>,
    pub(in crate::app) diffs: HashMap<String, ModRowDiff>,
    /// Height of everything drawn below the list, measured on the previous
    /// frame.
    ///
    /// The list is sized as "the window, less this", so the figure has to be the
    /// real one: a hardcoded guess that came out too small grew the window by
    /// the difference every frame, because a resizable egui window expands to
    /// fit its contents and never shrinks back. The naming lines and the
    /// overwrite warning appear conditionally, so there is no one number to
    /// guess -- measuring settles in a frame and cannot run away.
    pub(in crate::app) controls_height: f32,
}

/// Fold a mod name into a file-safe stem, keeping its capitalisation.
///
/// The name becomes three file names in a folder the user never types, so
/// spaces and punctuation are separators to normalise rather than characters
/// to carry through. Anything that is not a letter, digit, hyphen or
/// underscore becomes a hyphen, runs collapse, and the ends are trimmed --
/// kebab case, with the user's own casing left alone.
///
/// Underscores survive deliberately: `_P` marks a mod's priority, and folding
/// it to `-P` would leave a second one appended.
pub(in crate::app) fn sanitize_mod_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

impl ModExportDialog {
    /// The file stem the mod will be written under.
    ///
    /// `_P` is what gives an override container priority over the game's own,
    /// so it is part of the name rather than something the user can leave off.
    /// The game folds case before comparing, so a name already ending `_p` is
    /// left as it is.
    ///
    /// A name that sanitizes to nothing still gets a stem: the field is shown
    /// live beside the file names it produces, and a half-typed name that
    /// previewed as a bare `_P.utoc` read like a bug. Export stays disabled
    /// until the name is real.
    pub(in crate::app) fn stem(&self) -> String {
        let name = sanitize_mod_name(&self.name);
        let name = if name.is_empty() {
            "mod".to_owned()
        } else {
            name
        };
        if name.len() >= 2 && name[name.len() - 2..].eq_ignore_ascii_case("_p") {
            name
        } else {
            format!("{name}_P")
        }
    }

    pub(in crate::app) fn included(&self) -> impl Iterator<Item = &ModExportRow> {
        self.rows.iter().filter(|row| row.include)
    }

    /// Files that already exist where this would be written. A mod is three
    /// files plus its project sidecar, and only the container was ever guarded.
    pub(in crate::app) fn existing_files(&self) -> Vec<String> {
        let stem = self.stem();
        let destination = self.destination();
        MOD_FILE_EXTENSIONS
            .into_iter()
            .map(|extension| format!("{stem}.{extension}"))
            .filter(|name| destination.join(name).exists())
            .collect()
    }

    /// The directory the mod's files land in: the folder that was chosen, and
    /// nothing appended to it.
    ///
    /// The mod name names the *files*, never a directory. Deriving a folder
    /// from it too meant picking `Paks/~mods` and typing `mymod` wrote to
    /// `Paks/~mods/mymod/`, which is a level of nesting nobody asked for and
    /// which the engine's loader does not require — `~mods` is scanned
    /// recursively, so a triplet sitting directly in it is found either way.
    /// The default folder is already `Paks/~mods` (see `open_mod_review`), so
    /// the obvious export lands in the obvious place with no guessing here.
    pub(in crate::app) fn destination(&self) -> PathBuf {
        self.folder.clone()
    }
}

/// The directory mods are grouped under, and the export's default destination
/// inside the game's own `Paks`.
pub(in crate::app) const MODS_DIR: &str = "~mods";

/// Everything one exported mod is made of, in the order the dialog lists them.
/// The three the engine loads, plus the project sidecar that travels with them.
pub(in crate::app) const MOD_FILE_EXTENSIONS: [&str; 4] = ["utoc", "ucas", "pak", "baboon"];

/// What Export Mod just wrote, so the app can say what to do with it.
///
/// A mod is three files, and only the `.pak` looks like one. The status line
/// used to carry the instruction and no longer did; it also clears itself after
/// a few seconds, which is not long enough to act on.
pub(in crate::app) struct ExportedMod {
    pub(in crate::app) stem: String,
    pub(in crate::app) directory: PathBuf,
    pub(in crate::app) count: usize,
    pub(in crate::app) skipped: usize,
}

#[cfg(test)]
mod mod_export_tests;
