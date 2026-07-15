//! Find-in-tag state shared by the dialog, matcher, and field renderer.

/// Set of documents searched by the Find dialog.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum FindWithin {
    #[default]
    CurrentTag,
    OpenTags,
    AllTags,
}

impl FindWithin {
    /// User-facing scope name shown in the Find dialog.
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::CurrentTag => "Current Tag",
            Self::OpenTags => "Open Tags",
            Self::AllTags => "All Tags",
        }
    }
}

/// Which parts of a schema-backed field participate in Find.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum FindLookIn {
    #[default]
    FieldValues,
    Labels,
    Both,
}

impl FindLookIn {
    /// User-facing target name shown in the Find dialog.
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::FieldValues => "Field Values",
            Self::Labels => "Labels",
            Self::Both => "Both",
        }
    }

    /// Whether scalar field values participate in this target mode.
    pub(in crate::app) fn includes_values(self) -> bool {
        matches!(self, Self::FieldValues | Self::Both)
    }

    /// Whether field and container labels participate in this target mode.
    pub(in crate::app) fn includes_labels(self) -> bool {
        matches!(self, Self::Labels | Self::Both)
    }
}

/// Render surface on which one textual match occurs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::app) enum FindTargetKind {
    Label,
    Value,
}

/// One exact substring occurrence in one indexed tag field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct FindOccurrence {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) field_path: String,
    pub(in crate::app) kind: FindTargetKind,
    pub(in crate::app) text: String,
    pub(in crate::app) range: std::ops::Range<usize>,
}

/// Persistent state for the modeless Ctrl+F window.
#[derive(Default)]
pub(in crate::app) struct FindDialogState {
    pub(in crate::app) open: bool,
    pub(in crate::app) focus_query: bool,
    pub(in crate::app) query: String,
    pub(in crate::app) within: FindWithin,
    pub(in crate::app) look_in: FindLookIn,
    pub(in crate::app) match_case: bool,
    pub(in crate::app) whole_word: bool,
    pub(in crate::app) occurrences: Vec<FindOccurrence>,
    pub(in crate::app) active: Option<usize>,
    pub(in crate::app) all_request_id: u64,
    pub(in crate::app) all_signature: Option<String>,
    pub(in crate::app) all_closed_occurrences: Vec<FindOccurrence>,
    pub(in crate::app) all_order: Vec<String>,
    pub(in crate::app) searching: bool,
    pub(in crate::app) progress: Option<(usize, usize)>,
    pub(in crate::app) unreadable: usize,
}

impl FindDialogState {
    /// Currently selected occurrence, if its stored index remains valid.
    pub(in crate::app) fn active_occurrence(&self) -> Option<&FindOccurrence> {
        self.active.and_then(|index| self.occurrences.get(index))
    }

    /// Close Find and discard transient results while preserving its options.
    pub(in crate::app) fn close(&mut self) {
        self.open = false;
        self.active = None;
        self.occurrences.clear();
        self.searching = false;
        self.progress = None;
    }
}

/// Cloneable render-only Find data installed in egui temporary memory each frame.
#[derive(Clone)]
pub(in crate::app) struct FindRenderSnapshot {
    pub(in crate::app) query: String,
    pub(in crate::app) match_case: bool,
    pub(in crate::app) whole_word: bool,
    pub(in crate::app) active: Option<FindOccurrence>,
    pub(in crate::app) matching_cells: std::collections::HashSet<(String, String, FindTargetKind)>,
}

/// Identity of the Foundation field currently being painted.
#[derive(Clone)]
pub(in crate::app) struct FindRenderCell {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) field_path: String,
}
