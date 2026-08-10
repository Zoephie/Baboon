//! Which Campaign Evolved tag groups can actually be authored, and why not
//! when they cannot.
//! It owns the verdict and its reasoning; the wrapper bytes belong to
//! `blam-tags`, and where the answer is shown belongs to the UI modules.
//!
//! A group's Unreal identity is a **native** class,
//! `/Script/BlamSynchronization.Blam<PascalCase(group)>TagDataAsset`, compiled
//! into the game's binary. Nothing in a pak can add one, so "can I make a
//! `foo`?" is not a question about Baboon — it is a question about what the
//! binary already declares, and it has been answerable all along by reading the
//! mappings. It just was not being asked until someone tried and failed.

use super::*;

/// Why a group can or cannot be authored right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum GroupAuthorability {
    /// The game ships tags of this group, so a new one is cloned from one of
    /// them. The path with by far the most mileage on it.
    Donor { shipped: usize },
    /// Nothing shipped to clone, but the class adds nothing over
    /// `BlamTagDataAssetBase`, so the wrapper is built from the group alone.
    Derived,
    /// The class exists but declares properties that name other packages. A
    /// derived wrapper would be structurally valid and silently declare none of
    /// them, which is the failure cloning already had.
    NeedsDonor { properties: Vec<String> },
    /// No such class in the binary. Out of reach from a pak entirely, and not
    /// something more work on Baboon would fix.
    NoClass,
}

impl GroupAuthorability {
    pub(in crate::app) fn authorable(&self) -> bool {
        matches!(self, Self::Donor { .. } | Self::Derived)
    }

    /// One line for the group list, in the user's terms rather than the
    /// format's.
    pub(in crate::app) fn summary(&self) -> String {
        match self {
            Self::Donor { shipped: 1 } => "1 shipped tag to copy from".to_owned(),
            Self::Donor { shipped } => format!("{shipped} shipped tags to copy from"),
            Self::Derived => "none shipped, but this one can be built from scratch".to_owned(),
            Self::NeedsDonor { properties } => {
                let named: Vec<&str> = properties.iter().take(3).map(String::as_str).collect();
                let more = properties.len().saturating_sub(named.len());
                let list = named.join(", ");
                if more > 0 {
                    format!("nothing shipped to copy, and it needs {list} and {more} more")
                } else {
                    format!("nothing shipped to copy, and it needs {list}")
                }
            }
            Self::NoClass => "the game has no class for this group".to_owned(),
        }
    }
}

/// Work out, for one group, whether a tag of it can be created.
///
/// `shipped` is how many tags of the group the mounted paks already hold —
/// taken from the mount rather than from the definitions, because a group the
/// schemas describe and the game never shipped is exactly the case worth
/// distinguishing.
pub(in crate::app) fn group_authorability(
    group: &str,
    shipped: usize,
    usmap: &blam_tags::iostore::object::usmap::Usmap,
) -> GroupAuthorability {
    use blam_tags::iostore::asset::tag_package::{extra_properties, group_to_class};

    if shipped > 0 {
        return GroupAuthorability::Donor { shipped };
    }
    let class = group_to_class(group);
    // The class list comes from the mappings, which are dumped from the running
    // binary — so an absent class means the binary genuinely has none, not that
    // Baboon failed to look.
    if usmap.get(&class).is_none() {
        return GroupAuthorability::NoClass;
    }
    match extra_properties(&class, usmap) {
        Ok(properties) if properties.is_empty() => GroupAuthorability::Derived,
        Ok(properties) => GroupAuthorability::NeedsDonor { properties },
        // The class is declared but its schema chain does not resolve. Reported
        // as needing a donor with nothing named, rather than as having no class:
        // the class is there, and saying otherwise would be the more misleading
        // of the two wrong answers.
        Err(_) => GroupAuthorability::NeedsDonor {
            properties: Vec::new(),
        },
    }
}

/// How many tags of each group the mounted source holds, keyed by group tag.
///
/// Counts `all_entries` when a background scan has filled it and `entries`
/// otherwise, which is what every other whole-source question in Baboon does —
/// a container mount enumerates into `entries` and leaves `all_entries` empty.
pub(in crate::app) fn shipped_counts_by_group(source: &LoadedSourceData) -> HashMap<u32, usize> {
    let mut counts = HashMap::new();
    for entry in source.full_entry_set() {
        *counts.entry(entry.group_tag).or_insert(0) += 1;
    }
    counts
}

impl Baboon {
    /// Answer "can I make one of these?" for the group the New Tag dialog has
    /// selected, and cache it.
    ///
    /// Called when the group or the game changes, which is the only time the
    /// answer can move — it parses the whole mapping table, so it must not run
    /// per frame.
    pub(in crate::app) fn refresh_group_authorability(&mut self) {
        self.new_tag_dialog.authorability = None;
        // Only Campaign Evolved has native classes standing behind its groups.
        // Everywhere else a new tag is a file, and there is nothing to refuse.
        if self.new_tag_dialog.game != "haloce_evolved" {
            return;
        }
        let Some(group) = self
            .new_tag_dialog
            .groups
            .get(self.new_tag_dialog.selected_group)
        else {
            return;
        };
        let Ok(usmap) = blam_tags::iostore::object::usmap::Usmap::meteorite() else {
            return;
        };
        let shipped = self
            .source()
            .map(shipped_counts_by_group)
            .and_then(|counts| counts.get(&group.group_tag).copied())
            .unwrap_or(0);
        let verdict = group_authorability(&group.name, shipped, &usmap);
        self.new_tag_dialog.authorability = Some((verdict.authorable(), verdict.summary()));
    }
}

#[cfg(test)]
#[path = "../tests/group_report.rs"]
mod group_report_tests;
