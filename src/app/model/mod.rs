//! The application model: the state every feature reads.
//!
//! Open kits and which one is active, the live preferences, the default tag
//! names and the status line. Held apart from the features' own state so a
//! draw can read all of it while holding its feature's state mutably.

use super::*;

/// The application model: open kits and the active one, the live preferences,
/// the default tag names and the status line.
pub(in crate::app) struct Model {
    pub(in crate::app) default_names: TagNameIndex,
    /// Every open kit: the content store of the multi-kit model, each owning
    /// its source and all state scoped to it. **Never empty** — an unloaded
    /// Baboon holds one empty workspace kit, so readers of per-kit state need
    /// no "nothing loaded" special case. Cross-frame references use [`KitId`],
    /// never a position, since positions shift when a kit closes.
    pub(in crate::app) kits: Vec<Kit>,
    /// Index into `kits` of the kit the browser, tabs, and editor act on.
    /// Always a valid index; kept in range whenever `kits` changes.
    pub(in crate::app) active: usize,
    /// Monotonic [`KitId`] allocator; ids are never reused.
    pub(in crate::app) next_kit_id: u64,
    /// The live preferences: what Settings edits and every reader consults.
    /// `browser_mode` / `browser_sort` here are only the seed a new workspace
    /// starts from — each kit keeps its own — and [`Baboon::current_prefs`]
    /// takes the focused kit's when it writes them out.
    pub(in crate::app) prefs: GuiPrefs,
    pub(in crate::app) status: String,
}
