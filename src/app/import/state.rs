//! The single-tag import dialog's state and the profile fit it shows.

use super::*;

/// How a picked file has to be turned into a Campaign Evolved tag.
///
/// Import used to ask one question — "does this match the definition we ship?"
/// — and treat every answer short of yes as drift the user could wave through.
/// That conflates two unrelated situations. A tag saved by an older toolset
/// against a drifted Campaign Evolved layout really is safe to wave through.
/// A tag authored for *another game* is not: its root struct can agree while
/// nested structs disagree, so copying the bytes lands a tag the simulation
/// will read at the wrong offsets. Naming which of the two we are looking at
/// is what lets the gate refuse only the second.
pub(in crate::app) enum ImportMode {
    /// The file already carries Campaign Evolved's layout: copy its bytes.
    Native {
        comparison: Option<blam_tags::LayoutComparison>,
        /// Wave through benign field-metadata drift. Offered only when no
        /// other profile claims the file either — it is an answer to "this
        /// toolset wrote the struct slightly differently", never to "this is a
        /// Halo Reach tag".
        import_anyway: bool,
    },
    /// The file matches another game's profile, so its bytes have to be
    /// converted before they can land. `draft` is the analyzed conversion once
    /// one exists; `None` means it has not been run, or cannot be.
    Convert {
        source_game: String,
        draft: Option<TagConversionDraft>,
    },
}

/// How well an imported tag's own layout fits one game's definition of its
/// group.
///
/// Deliberately not `blam_tags::LayoutSeverity`, which compares root structs
/// only. That is the right cost for asking "roughly, is this our group?" and the
/// wrong answer for "can these bytes be copied?" — a Halo Reach animation graph
/// earns a clean `Match` against Campaign Evolved because their root structs are
/// identical, while four nested structs are the wrong size.
#[derive(Debug)]
pub(in crate::app) enum ProfileFit {
    /// Wire-identical all the way down. Bytes written against this profile can
    /// be read as this game's version of the group without reinterpretation.
    Identical,
    /// The same group, but the shapes diverge somewhere below the root. Carries
    /// where, because "differs" without a location is not actionable on a group
    /// the size of `scenario`.
    Diverges(String),
    /// Not this game's version of this group at all.
    WrongGroup,
}

impl ProfileFit {
    pub(in crate::app) fn is_identical(&self) -> bool {
        matches!(self, ProfileFit::Identical)
    }
}

/// Import-a-tag-file dialog for a Campaign Evolved container source. Owns the
/// parsed imported `TagFile` (moved out on confirm) and how it has to be
/// landed. Not `Clone` — `TagFile` isn't cloneable.
pub(in crate::app) struct ImportTagDialog {
    /// Workspace this was raised from. The confirm applies against the active
    /// kit, and a modeless dialog outlives the frame that opened it, so the
    /// user can focus another game in between; resolving this first is what
    /// keeps the action on the workspace it was started in.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) source_path: PathBuf,
    /// Pre-filled container folder (empty for the root); the leaf name is `name`.
    pub(in crate::app) folder_rel: String,
    pub(in crate::app) name: String,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) group_name: String,
    pub(in crate::app) extension: String,
    /// The parsed imported tag; `take()`n when the user confirms.
    pub(in crate::app) tag: Option<TagFile>,
    /// How this file has to be landed.
    pub(in crate::app) mode: ImportMode,
    /// Every profile that defines this group, with how the imported tag's own
    /// layout fits it. Evidence rather than a decision: it seeds `mode` and
    /// lets the user correct the guess when the file is unusual.
    pub(in crate::app) profile_verdicts: Vec<(String, ProfileFit)>,
    pub(in crate::app) error: Option<String>,
}

pub(in crate::app) struct PendingImport {
    /// Workspace this was raised from. The confirm applies against the active
    /// kit, and a modeless dialog outlives the frame that opened it, so the
    /// user can focus another game in between; resolving this first is what
    /// keeps the action on the workspace it was started in.
    pub(in crate::app) kit: KitId,
    pub(in crate::app) tag: TagFile,
    pub(in crate::app) target_key: String,
}
