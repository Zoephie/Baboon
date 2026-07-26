//! One loaded editing kit (tag source) and the state scoped to it.
//! It owns kit identity and per-source state; global preferences, dialogs, and process-level services belong on [`Baboon`].

use super::*;

/// Stable, never-reused identity for a loaded kit.
///
/// Allocated from a monotonic counter on [`Baboon`], never from a position in
/// `kits`. This is the load-bearing invariant of the multi-kit model: a stale
/// id left behind by a background job or a layout reference resolves to `None`
/// after its kit closes, where a positional index would silently retarget
/// whichever kit slid into that slot.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct KitId(pub(super) u64);

/// One loaded editing kit: a tag source plus the state scoped to it.
///
/// Baboon was historically single-source, with every per-source and per-tag map
/// living flat on [`Baboon`] and keyed by a bare tag-key string. Those fields
/// move here as the migration proceeds, so that several kits can be resident at
/// once without their documents, caches, and indices aliasing one another.
pub(super) struct Kit {
    pub(super) id: KitId,
    /// The loaded source and its source-relative browser/index state.
    pub(super) source: LoadedSourceData,
    /// This kit's tag-name index: the source's own names merged over the
    /// application defaults. Per-kit because group and field naming differs
    /// between games, and split view renders two kits in the same frame.
    pub(super) names: TagNameIndex,
}

impl Baboon {
    /// Position of `id` in `kits`, for call sites that must borrow the kit and
    /// another field of `self` at the same time. Those cannot go through
    /// [`Baboon::kit_mut`] — a method borrows all of `self` — so they index
    /// `self.kits[idx]` directly and let the borrow checker see the two fields
    /// as disjoint.
    pub(super) fn kit_index(&self, id: KitId) -> Option<usize> {
        self.kits.iter().position(|kit| kit.id == id)
    }

    pub(super) fn kit(&self, id: KitId) -> Option<&Kit> {
        self.kits.iter().find(|kit| kit.id == id)
    }

    pub(super) fn kit_mut(&mut self, id: KitId) -> Option<&mut Kit> {
        self.kits.iter_mut().find(|kit| kit.id == id)
    }

    pub(super) fn active_kit_index(&self) -> Option<usize> {
        self.kit_index(self.active_kit?)
    }

    pub(super) fn active_kit(&self) -> Option<&Kit> {
        self.kit(self.active_kit?)
    }

    pub(super) fn active_kit_mut(&mut self) -> Option<&mut Kit> {
        self.kit_mut(self.active_kit?)
    }

    /// The active kit's source. Replaces the former `Baboon::source` field so
    /// the many read-only call sites keep their shape; sites that also need a
    /// mutable borrow of another field must use [`Baboon::active_kit_index`].
    pub(super) fn source(&self) -> Option<&LoadedSourceData> {
        Some(&self.active_kit()?.source)
    }

    pub(super) fn source_mut(&mut self) -> Option<&mut LoadedSourceData> {
        Some(&mut self.active_kit_mut()?.source)
    }

    /// Install a freshly loaded source, replacing whatever was loaded before.
    /// Multi-kit loading (add-a-kit rather than replace) arrives with the kit
    /// strip; until then this preserves the single-source behavior exactly.
    pub(super) fn install_loaded_source(&mut self, source: LoadedSourceData) -> KitId {
        let id = KitId(self.next_kit_id);
        self.next_kit_id = self.next_kit_id.wrapping_add(1);
        let mut names = source.names.clone();
        names.merge_missing(self.default_names.clone());
        self.kits = vec![Kit { id, source, names }];
        self.active_kit = Some(id);
        id
    }

    /// The active kit's tag-name index, falling back to the application
    /// defaults when no kit is loaded. Read-only sites use this; anything that
    /// also needs a mutable sibling borrow goes through the kit index.
    pub(super) fn names(&self) -> &TagNameIndex {
        match self.active_kit() {
            Some(kit) => &kit.names,
            None => &self.default_names,
        }
    }
}
