//! The theme in use: which one the preferences chose, what resolving it
//! reported, and the once-a-second check that reloads it when one of its
//! files changes. Lives on the UI thread.

use super::file::Report;
use super::library::{Fingerprint, ThemeEntry, ThemeLibrary};
use super::{BuiltIn, ThemeColors, install};
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How often the active theme's files are checked for an edit.
pub(in crate::app) const RELOAD_CHECK: Duration = Duration::from_secs(1);

/// The folder user themes live in, under Baboon's data directory, so the
/// installed and portable modes each have their own.
pub(in crate::app) fn themes_folder() -> PathBuf {
    crate::core::storage::data_path("themes")
}

/// The theme applied, and what it was read from.
#[derive(Clone, Debug)]
struct Applied {
    id: String,
    /// The built-in shown when the theme cannot be.
    slot: BuiltIn,
    reports: Vec<Report>,
    /// Each file it was read from, with its fingerprint when read.
    files: Vec<(PathBuf, Option<Fingerprint>)>,
    /// Why the theme is not what is drawn: it could not be applied, or its
    /// last reload failed and the colours before it are kept.
    failed: Option<Report>,
    /// What applying it installed, for a preview to give back.
    colors: ThemeColors,
}

/// The theme chosen and what came of it.
#[derive(Default)]
pub(in crate::app) struct ThemeState {
    folder: Option<PathBuf>,
    library: Option<(ThemeLibrary, Instant)>,
    applied: Option<Applied>,
    last_check: Option<Instant>,
    status: Option<String>,
    /// The theme a menu is previewing, drawn in place of the applied one.
    previewing: Option<String>,
}

impl ThemeState {
    /// A state reading user themes from `folder`, or none.
    pub(in crate::app) fn new(folder: Option<PathBuf>) -> Self {
        Self {
            folder,
            ..Self::default()
        }
    }

    fn scan(&self) -> ThemeLibrary {
        self.folder
            .as_deref()
            .map(ThemeLibrary::scan)
            .unwrap_or_default()
    }

    /// The library, read now if never read.
    fn library(&mut self) -> &ThemeLibrary {
        if self.library.is_none() {
            self.library = Some((self.scan(), Instant::now()));
        }
        &self.library.as_ref().unwrap().0
    }

    /// Applies the theme `id`, chosen for an appearance whose built-in is
    /// `slot`. Applying the theme already applied only runs the reload check,
    /// at most once a [`RELOAD_CHECK`]; a theme that cannot be applied shows
    /// `slot` and says so in the status line.
    pub(in crate::app) fn apply(&mut self, id: &str, slot: BuiltIn, now: Instant) {
        let same = self
            .applied
            .as_ref()
            .is_some_and(|applied| applied.id == id && applied.slot == slot);
        if same {
            if self
                .last_check
                .is_none_or(|at| now.duration_since(at) >= RELOAD_CHECK)
            {
                self.last_check = Some(now);
                if self.files_changed() {
                    let library = self.scan();
                    self.reload_with(library, now);
                }
            }
            return;
        }
        self.last_check = Some(now);
        let resolved = if BuiltIn::from_id(id).is_some() {
            ThemeLibrary::default().resolve(id)
        } else {
            self.library().resolve(id)
        };
        match resolved {
            Ok(resolved) => {
                install(&resolved.colors);
                self.applied = Some(Applied {
                    id: id.to_owned(),
                    slot,
                    reports: resolved.reports,
                    files: fingerprinted(resolved.files),
                    failed: None,
                    colors: resolved.colors,
                });
            }
            Err(report) => {
                install(&slot.colors());
                self.status = Some(format!(
                    "The theme {id} could not be applied ({report}); showing {}",
                    slot.name()
                ));
                self.applied = Some(Applied {
                    id: id.to_owned(),
                    slot,
                    reports: Vec::new(),
                    files: self
                        .library()
                        .listed(id)
                        .map(|listed| fingerprinted(vec![listed.path.clone()]))
                        .unwrap_or_default(),
                    failed: Some(report),
                    colors: slot.colors(),
                });
            }
        }
    }

    fn files_changed(&self) -> bool {
        self.applied.as_ref().is_some_and(|applied| {
            applied
                .files
                .iter()
                .any(|(path, fingerprint)| Fingerprint::of(path) != *fingerprint)
        })
    }

    /// Reads the themes folder again and the applied theme from it, as the
    /// Reload button does.
    pub(in crate::app) fn reload(&mut self, now: Instant) {
        let library = self.scan();
        self.reload_with(library, now);
    }

    /// Takes `library` as the themes folder's and applies the applied theme
    /// from it. A theme that no longer resolves keeps the colours drawn, and
    /// reports why.
    pub(in crate::app) fn reload_with(&mut self, library: ThemeLibrary, now: Instant) {
        self.library = Some((library, now));
        let Some(applied) = self.applied.clone() else {
            return;
        };
        if BuiltIn::from_id(&applied.id).is_some() {
            return;
        }
        match self.library().resolve(&applied.id) {
            Ok(resolved) => {
                install(&resolved.colors);
                self.applied = Some(Applied {
                    reports: resolved.reports,
                    files: fingerprinted(resolved.files),
                    failed: None,
                    colors: resolved.colors,
                    ..applied
                });
            }
            Err(report) => {
                self.status = Some(format!(
                    "The theme {} could not be reloaded ({report}); its last colours are kept",
                    applied.id
                ));
                let files = self
                    .library()
                    .listed(&applied.id)
                    .map(|listed| fingerprinted(vec![listed.path.clone()]))
                    .unwrap_or(applied.files.clone());
                self.applied = Some(Applied {
                    failed: Some(report),
                    files,
                    ..applied
                });
            }
        }
    }

    /// Every theme a menu offers. The folder is read again when the list is
    /// older than a [`RELOAD_CHECK`], so a file added while Settings is open
    /// appears.
    pub(in crate::app) fn entries(&mut self, now: Instant) -> Vec<ThemeEntry> {
        if self
            .library
            .as_ref()
            .is_none_or(|(_, at)| now.duration_since(*at) >= RELOAD_CHECK)
        {
            self.library = Some((self.scan(), now));
        }
        self.library().entries()
    }

    /// Draws the theme `id` while a menu's pointer rests on it, or, given
    /// `None`, gives back the applied theme's colours if a preview had
    /// replaced them. A theme that cannot be resolved is not previewed.
    pub(in crate::app) fn preview(&mut self, id: Option<&str>) {
        match id {
            Some(id) if self.previewing.as_deref() != Some(id) => {
                let resolved = if BuiltIn::from_id(id).is_some() {
                    ThemeLibrary::default().resolve(id)
                } else {
                    self.library().resolve(id)
                };
                if let Ok(resolved) = resolved {
                    install(&resolved.colors);
                }
                self.previewing = Some(id.to_owned());
            }
            Some(_) => {}
            None => {
                if self.previewing.take().is_some()
                    && let Some(applied) = &self.applied
                {
                    install(&applied.colors);
                }
            }
        }
    }

    /// What the active theme reported, and the folder's own problems.
    pub(in crate::app) fn problems(&mut self) -> Vec<Report> {
        let mut problems = Vec::new();
        if let Some(applied) = &self.applied {
            problems.extend(applied.failed.iter().cloned());
            problems.extend(applied.reports.iter().cloned());
        }
        problems.extend(self.library().reports.iter().cloned());
        problems
    }

    /// The id of the theme the preferences chose last.
    pub(in crate::app) fn applied_id(&self) -> Option<&str> {
        self.applied.as_ref().map(|applied| applied.id.as_str())
    }

    /// The library as last read.
    pub(in crate::app) fn current_library(&mut self) -> &ThemeLibrary {
        self.library()
    }

    /// A status line to show once, if any.
    pub(in crate::app) fn take_status(&mut self) -> Option<String> {
        self.status.take()
    }
}

fn fingerprinted(files: Vec<PathBuf>) -> Vec<(PathBuf, Option<Fingerprint>)> {
    files
        .into_iter()
        .map(|path| {
            let fingerprint = Fingerprint::of(&path);
            (path, fingerprint)
        })
        .collect()
}

std::thread_local! {
    static STATE: RefCell<Option<ThemeState>> = const { RefCell::new(None) };
}

/// Runs `f` on the UI thread's theme state, reading user themes from
/// [`themes_folder`].
pub(in crate::app) fn with_state<R>(f: impl FnOnce(&mut ThemeState) -> R) -> R {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = state.get_or_insert_with(|| ThemeState::new(Some(themes_folder())));
        f(state)
    })
}

/// Points the UI thread's theme state at `folder`, for a test that applies
/// themes from it.
#[cfg(test)]
pub(in crate::app) fn use_folder(folder: PathBuf) {
    STATE.with(|state| *state.borrow_mut() = Some(ThemeState::new(Some(folder))));
}
