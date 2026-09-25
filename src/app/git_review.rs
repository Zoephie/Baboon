//! Read-only Git repository discovery and semantic tag comparison state.

use super::*;

pub(in crate::app) const GIT_REVIEW_KEY: &str = "tool:git_review";
pub(in crate::app) const GIT_REVIEW_TITLE: &str = "Git Review";

/// Git always reports repository-relative paths with `/`. Build the working
/// tree path one component at a time so the filesystem receives native path
/// components without changing the string later passed back to Git.
fn git_worktree_path(repo: &Path, git_path: &str) -> PathBuf {
    git_path
        .split('/')
        .filter(|component| !component.is_empty())
        .fold(repo.to_path_buf(), |mut path, component| {
            path.push(component);
            path
        })
}

pub(in crate::app) fn native_git_display_path(git_path: &str) -> String {
    #[cfg(windows)]
    {
        git_path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        git_path.to_owned()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) enum GitReviewSelection {
    Local,
    Commit(String),
}

impl Default for GitReviewSelection {
    fn default() -> Self {
        Self::Local
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct GitReviewCommit {
    pub(in crate::app) hash: String,
    pub(in crate::app) short_hash: String,
    pub(in crate::app) author: String,
    pub(in crate::app) date: String,
    pub(in crate::app) subject: String,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct GitReviewFile {
    pub(in crate::app) status: String,
    pub(in crate::app) path: String,
    pub(in crate::app) group_tag: u32,
}

#[derive(Default)]
pub(in crate::app) struct GitReviewState {
    pub(in crate::app) repo_root: Option<PathBuf>,
    pub(in crate::app) branch: String,
    pub(in crate::app) commits: Vec<GitReviewCommit>,
    pub(in crate::app) local_files: Vec<GitReviewFile>,
    pub(in crate::app) files: Vec<GitReviewFile>,
    pub(in crate::app) selection: GitReviewSelection,
    pub(in crate::app) selected_path: Option<String>,
    pub(in crate::app) results: Option<TagDiffResults>,
    /// A double-clicked working tag waiting for the tiled layout to be restored.
    pub(in crate::app) pending_open: Option<String>,
    pub(in crate::app) filters: TagDiffFilters,
    pub(in crate::app) swapped: bool,
    pub(in crate::app) commit_filter: String,
    pub(in crate::app) filter: String,
    pub(in crate::app) error: Option<String>,
    /// The newest Git job started for this review; older results are dropped.
    pub(in crate::app) request: u64,
    /// A Git job is running.
    pub(in crate::app) loading: bool,
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("Could not run Git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if message.is_empty() {
            format!("Git exited with {}", output.status)
        } else {
            message
        })
    }
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    git_output(root, args).map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
}

fn git_text_untrimmed(root: &Path, args: &[&str]) -> Result<String, String> {
    git_output(root, args).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn tag_group_for_path(path: &str) -> Option<u32> {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(extension_to_group_tag)
}

fn parse_commits(text: &str) -> Vec<GitReviewCommit> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(5, '\t');
            Some(GitReviewCommit {
                hash: fields.next()?.to_owned(),
                short_hash: fields.next()?.to_owned(),
                date: fields.next()?.to_owned(),
                author: fields.next()?.to_owned(),
                subject: fields.next()?.to_owned(),
            })
        })
        .collect()
}

fn parse_name_status(text: &str) -> Vec<GitReviewFile> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let status = fields.next()?.trim();
            let first = fields.next()?;
            let path = fields.next().unwrap_or(first);
            Some(GitReviewFile {
                status: status.chars().next().unwrap_or('?').to_string(),
                path: path.to_owned(),
                group_tag: tag_group_for_path(path)?,
            })
        })
        .collect()
}

fn parse_local_status(text: &str) -> Vec<GitReviewFile> {
    text.lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let status = line[..2].trim();
            let raw_path = line[3..].trim();
            let path = raw_path
                .rsplit_once(" -> ")
                .map_or(raw_path, |(_, to)| to)
                .trim_matches('"');
            Some(GitReviewFile {
                status: if status == "??" { "?" } else { status }.to_owned(),
                path: path.to_owned(),
                group_tag: tag_group_for_path(path)?,
            })
        })
        .collect()
}

fn retain_source_tags(files: &mut Vec<GitReviewFile>, repo: &Path, source_root: &Path) {
    files.retain(|file| git_worktree_path(repo, &file.path).starts_with(source_root));
}

fn sort_files_by_full_path(files: &mut [GitReviewFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

fn can_restore_review_selection(
    previous_repo: Option<&Path>,
    previous_selection: &GitReviewSelection,
    repo: &Path,
    commits: &[GitReviewCommit],
) -> bool {
    previous_repo == Some(repo)
        && match previous_selection {
            GitReviewSelection::Local => true,
            GitReviewSelection::Commit(hash) => commits.iter().any(|commit| &commit.hash == hash),
        }
}

/// What a Git Review job needs from its kit, captured on the UI thread.
#[derive(Clone)]
pub(in crate::app) struct GitReviewKit {
    source_root: PathBuf,
    definitions_root: PathBuf,
    game: Option<String>,
}

/// The part of [`GitReviewState`] that Git decides. A job takes a copy to a
/// worker, runs Git and the tag reads against it there, and the finished copy
/// replaces the state's in one go.
#[derive(Clone, Default)]
pub(in crate::app) struct GitReviewView {
    pub(in crate::app) repo_root: Option<PathBuf>,
    pub(in crate::app) branch: String,
    pub(in crate::app) commits: Vec<GitReviewCommit>,
    pub(in crate::app) local_files: Vec<GitReviewFile>,
    pub(in crate::app) files: Vec<GitReviewFile>,
    pub(in crate::app) selection: GitReviewSelection,
    pub(in crate::app) selected_path: Option<String>,
    pub(in crate::app) results: Option<TagDiffResults>,
    pub(in crate::app) error: Option<String>,
}

/// A Git Review job for a worker.
pub(in crate::app) enum GitReviewJob {
    Refresh,
    SelectRevision(GitReviewSelection),
    SelectFile(String),
}

impl GitReviewView {
    fn run(&mut self, kit: &GitReviewKit, job: GitReviewJob) {
        match job {
            GitReviewJob::Refresh => self.refresh(kit),
            GitReviewJob::SelectRevision(selection) => self.select_revision(kit, selection),
            GitReviewJob::SelectFile(path) => self.select_file(kit, path),
        }
    }

    fn refresh(&mut self, kit: &GitReviewKit) {
        let source_root = kit.source_root.as_path();
        let previous_repo = self.repo_root.clone();
        let previous_selection = self.selection.clone();
        let previous_path = self.selected_path.clone();
        let result = (|| {
            // Git for Windows prints the worktree root with `/` separators.
            // Keep Git's relative file paths untouched, but store the root in
            // native form for shell handoffs such as File Explorer.
            let repo = PathBuf::from(native_git_display_path(&git_text(
                source_root,
                &["rev-parse", "--show-toplevel"],
            )?));
            let branch = git_text(&repo, &["branch", "--show-current"])?;
            let log = git_text(
                &repo,
                &["log", "-n", "100", "--format=%H%x09%h%x09%cs%x09%an%x09%s"],
            )?;
            let mut files = parse_local_status(&git_text_untrimmed(
                &repo,
                &["status", "--short", "--untracked-files=all"],
            )?);
            retain_source_tags(&mut files, &repo, source_root);
            sort_files_by_full_path(&mut files);
            Ok::<_, String>((repo, branch, parse_commits(&log), files))
        })();

        match result {
            Ok((repo, branch, commits, files)) => {
                let restore_previous = can_restore_review_selection(
                    previous_repo.as_deref(),
                    &previous_selection,
                    &repo,
                    &commits,
                );
                let selection = if restore_previous {
                    previous_selection
                } else {
                    GitReviewSelection::Local
                };
                self.repo_root = Some(repo);
                self.branch = branch;
                self.commits = commits;
                self.local_files = files.clone();
                self.files = files;
                self.selection = GitReviewSelection::Local;
                self.selected_path = None;
                self.results = None;
                self.error = None;
                if selection != GitReviewSelection::Local {
                    self.select_revision(kit, selection);
                    if self.error.is_some() {
                        self.selection = GitReviewSelection::Local;
                        self.files = self.local_files.clone();
                        return;
                    }
                }
                if restore_previous
                    && let Some(path) = previous_path
                    && self.error.is_none()
                    && self.files.iter().any(|file| file.path == path)
                {
                    self.select_file(kit, path);
                }
            }
            Err(error) => {
                self.repo_root = None;
                self.error = Some(error);
            }
        }
    }

    fn select_revision(&mut self, kit: &GitReviewKit, selection: GitReviewSelection) {
        let Some(repo) = self.repo_root.clone() else {
            return;
        };
        let mut files = match &selection {
            GitReviewSelection::Local => {
                git_text_untrimmed(&repo, &["status", "--short", "--untracked-files=all"])
                    .map(|text| parse_local_status(&text))
            }
            GitReviewSelection::Commit(hash) => git_text(
                &repo,
                &[
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "--name-status",
                    "-r",
                    hash,
                ],
            )
            .map(|text| parse_name_status(&text)),
        };
        if let Ok(files) = &mut files {
            retain_source_tags(files, &repo, &kit.source_root);
        }
        let local_selected = selection == GitReviewSelection::Local;
        if local_selected && let Ok(files) = &mut files {
            sort_files_by_full_path(files);
        }
        self.selection = selection;
        self.selected_path = None;
        self.results = None;
        match files {
            Ok(files) => {
                if local_selected {
                    self.local_files = files.clone();
                }
                self.files = files;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn select_file(&mut self, kit: &GitReviewKit, path: String) {
        let result = self.comparison(kit, &path);
        self.selected_path = Some(path);
        match result {
            Ok(results) => {
                self.results = Some(results);
                self.error = None;
            }
            Err(error) => {
                self.results = None;
                self.error = Some(error);
            }
        }
    }

    fn comparison(&self, kit: &GitReviewKit, path: &str) -> Result<TagDiffResults, String> {
        let repo = self
            .repo_root
            .as_ref()
            .ok_or_else(|| "No Git repository is loaded.".to_owned())?;
        let file = self
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| "The selected tag is no longer in this change set.".to_owned())?;
        let definitions_root = kit.definitions_root.as_path();
        let game = kit.game.as_deref();
        let load_revision = |revision: &str| -> Result<Option<TagFile>, String> {
            let object = format!("{revision}:{path}");
            let exists = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["cat-file", "-e", &object])
                .output()
                .map_err(|error| format!("Could not run Git: {error}"))?;
            if !exists.status.success() {
                return Ok(None);
            }
            let bytes = git_output(repo, &["show", &object])?;
            crate::source::read_tag_from_bytes(&bytes, game, Some(definitions_root), file.group_tag)
                .map(Some)
                .map_err(|error| format!("Could not parse {path} at {revision}: {error}"))
        };
        let load_working = || -> Result<Option<TagFile>, String> {
            let absolute = git_worktree_path(repo, path);
            if !absolute.is_file() {
                return Ok(None);
            }
            crate::source::read_tag_at_path(&absolute, game, Some(definitions_root), file.group_tag)
                .map(Some)
                .map_err(|error| format!("Could not parse working tag {path}: {error}"))
        };
        let (before, after) = match &self.selection {
            GitReviewSelection::Local => (load_revision("HEAD")?, load_working()?),
            GitReviewSelection::Commit(hash) => {
                let parent = git_text(repo, &["rev-list", "--parents", "-n", "1", hash])?
                    .split_whitespace()
                    .nth(1)
                    .map(str::to_owned);
                let before = parent.as_deref().map(&load_revision).transpose()?.flatten();
                (before, load_revision(hash)?)
            }
        };
        Ok(
            crate::app::ui::tag_compare::comparison_results_with_missing(
                before.as_ref(),
                after.as_ref(),
            ),
        )
    }
}

impl GitReviewState {
    fn view(&self) -> GitReviewView {
        GitReviewView {
            repo_root: self.repo_root.clone(),
            branch: self.branch.clone(),
            commits: self.commits.clone(),
            local_files: self.local_files.clone(),
            files: self.files.clone(),
            selection: self.selection.clone(),
            selected_path: self.selected_path.clone(),
            results: self.results.clone(),
            error: self.error.clone(),
        }
    }

    fn apply(&mut self, view: GitReviewView) {
        self.repo_root = view.repo_root;
        self.branch = view.branch;
        self.commits = view.commits;
        self.local_files = view.local_files;
        self.files = view.files;
        self.selection = view.selection;
        self.selected_path = view.selected_path;
        self.results = view.results;
        self.error = view.error;
    }
}

impl Baboon {
    pub(super) fn open_git_review(&mut self, ctx: &egui::Context) {
        let kit = self.active;
        if !matches!(
            self.kits[kit].source.as_ref().map(|source| &source.source),
            Some(TagSource::LooseFolder { .. })
        ) {
            self.status = "Git Review is available for folder-based editing kits".to_owned();
            return;
        }
        self.kits[kit].open_tag_pane(GIT_REVIEW_KEY);
        self.run_git_review_job(kit, GitReviewJob::Refresh, ctx);
    }

    /// Run Git for the review on a worker: `status` over a whole kit, and the
    /// reads and diff behind a selected file, are too slow for a frame.
    ///
    /// Each job starts from the state as it stands and replaces it when it
    /// finishes. Only the newest job's result is applied, so a slow read for
    /// a file the user has already clicked past cannot land over a later one.
    pub(in crate::app) fn run_git_review_job(
        &mut self,
        kit_index: usize,
        job: GitReviewJob,
        ctx: &egui::Context,
    ) {
        let Some((source_root, definitions_root, game)) = self.kits[kit_index]
            .source
            .as_ref()
            .and_then(|source| match &source.source {
                TagSource::LooseFolder {
                    root,
                    definitions_root,
                    ..
                } => Some((root.clone(), definitions_root.clone(), source.game.clone())),
                _ => None,
            })
        else {
            self.kits[kit_index].git_review.error =
                Some("Git Review requires a folder-based editing kit.".to_owned());
            return;
        };
        let review_kit = GitReviewKit {
            source_root,
            definitions_root,
            game,
        };
        let kit = self.kits[kit_index].id;
        let state = &mut self.kits[kit_index].git_review;
        state.request += 1;
        state.loading = true;
        let request = state.request;
        let mut view = state.view();
        spawn_worker(
            &self.tx,
            ctx,
            move || {
                view.run(&review_kit, job);
                WorkerMessage::GitReviewUpdated {
                    kit,
                    request,
                    view: Ok(view),
                }
            },
            move |error| WorkerMessage::GitReviewUpdated {
                kit,
                request,
                view: Err(error),
            },
        );
    }

    /// Applies `WorkerMessage::GitReviewUpdated` if it is the newest job.
    pub(in crate::app) fn handle_git_review_updated(
        &mut self,
        kit: KitId,
        request: u64,
        view: Result<GitReviewView, String>,
    ) -> bool {
        let Some(kit_index) = self.kit_index(kit) else {
            return false;
        };
        let state = &mut self.kits[kit_index].git_review;
        if state.request != request {
            return false;
        }
        state.loading = false;
        match view {
            Ok(view) => state.apply(view),
            Err(error) => state.error = Some(error),
        }
        false
    }

    pub(in crate::app) fn open_git_review_file(&mut self, kit: usize, path: &str) {
        let Some(repo) = self.kits[kit].git_review.repo_root.as_ref() else {
            return;
        };
        let absolute = git_worktree_path(repo, path);
        if !absolute.is_file() {
            self.status = format!("Cannot open deleted tag {}", native_git_display_path(path));
            return;
        }
        // The review only lists files under the kit's root as the kit spells
        // it (`retain_source_tags`), so this is the path the kit's own scan
        // produced, and its key is the scan's key. This used to canonicalize
        // the path against every entry in the kit, twice, per click.
        let key = format!("file:{}", absolute.display());
        if self.kits[kit].entry_for_key(&key).is_none() {
            let new_entry = self.kits[kit].source.as_ref().and_then(|source| {
                let TagSource::LooseFolder { root, .. } = &source.source else {
                    return None;
                };
                loose_file_entry(root, &absolute, &source.names)
                    .ok()
                    .flatten()
            });
            let Some(entry) = new_entry else {
                self.status = format!(
                    "Cannot find tag {} in the loaded editing kit",
                    native_git_display_path(path)
                );
                return;
            };
            let folder_seeds = self.kits[kit].folder_seeds();
            if let Some(source) = self.kits[kit].source.as_mut() {
                source.upsert_entry(entry, &folder_seeds);
            }
            self.kits[kit].generation = self.kits[kit].generation.wrapping_add(1);
        }
        self.kits[kit].git_review.pending_open = Some(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parsers_hide_non_tags_and_keep_tag_types() {
        let files =
            parse_local_status(" M tags/a.weapon\nR  old/path -> tags/b.weapon\n?? notes.txt\n");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, "M");
        assert_eq!(files[0].path, "tags/a.weapon");
        assert_eq!(files[0].group_tag, u32::from_be_bytes(*b"weap"));
        assert_eq!(files[1].path, "tags/b.weapon");
    }

    #[test]
    fn parses_commit_name_status() {
        let files =
            parse_name_status("M\ttags/a.weapon\nR100\told.weapon\tnew.weapon\nD\tREADME.md\n");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, "M");
        assert_eq!(files[1].path, "new.weapon");
    }

    #[test]
    fn git_paths_become_native_worktree_paths_without_changing_git_strings() {
        let repo = Path::new("repo-root");
        let path = git_worktree_path(repo, "tags/objects/rifle.weapon");
        assert_eq!(path, repo.join("tags").join("objects").join("rifle.weapon"));
        #[cfg(windows)]
        assert_eq!(
            native_git_display_path("tags/objects/rifle.weapon"),
            r"tags\objects\rifle.weapon"
        );
        #[cfg(not(windows))]
        assert_eq!(
            native_git_display_path("tags/objects/rifle.weapon"),
            "tags/objects/rifle.weapon"
        );
    }

    #[test]
    fn git_repository_root_uses_native_separators_for_shell_handoffs() {
        let root = PathBuf::from(native_git_display_path("C:/Program Files/H2CE"));
        #[cfg(windows)]
        assert_eq!(root.to_string_lossy(), r"C:\Program Files\H2CE");
        #[cfg(not(windows))]
        assert_eq!(root.to_string_lossy(), "C:/Program Files/H2CE");
    }

    #[test]
    fn local_changes_sort_by_full_path_not_status() {
        let mut files = parse_local_status(
            "?? tags/z.weapon\n M tags/b.weapon\n?? tags/a.weapon\n D tags/c.weapon\n",
        );
        sort_files_by_full_path(&mut files);
        assert_eq!(
            files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            [
                "tags/a.weapon",
                "tags/b.weapon",
                "tags/c.weapon",
                "tags/z.weapon"
            ]
        );
    }

    #[test]
    fn refresh_restores_only_revisions_still_in_the_same_repository() {
        let repo = Path::new("repo");
        let commit = GitReviewCommit {
            hash: "abc123".to_owned(),
            short_hash: "abc123".to_owned(),
            author: String::new(),
            date: String::new(),
            subject: String::new(),
        };
        assert!(can_restore_review_selection(
            Some(repo),
            &GitReviewSelection::Local,
            repo,
            &[],
        ));
        assert!(can_restore_review_selection(
            Some(repo),
            &GitReviewSelection::Commit(commit.hash.clone()),
            repo,
            &[commit],
        ));
        assert!(!can_restore_review_selection(
            Some(repo),
            &GitReviewSelection::Commit("missing".to_owned()),
            repo,
            &[],
        ));
        assert!(!can_restore_review_selection(
            Some(Path::new("other")),
            &GitReviewSelection::Local,
            repo,
            &[],
        ));
    }

    /// Jobs finish in any order. A slow comparison for a file the user has
    /// already clicked past must not land over the newer one.
    #[test]
    fn an_older_git_review_result_is_dropped() {
        let mut app = Baboon::for_test();
        let kit = app.kits[0].id;
        app.kits[0].git_review.request = 2;
        app.kits[0].git_review.loading = true;
        let view = |path: &str| GitReviewView {
            selected_path: Some(path.to_owned()),
            ..Default::default()
        };

        app.handle_git_review_updated(kit, 1, Ok(view("first.weapon")));
        let state = &app.kits[0].git_review;
        assert_eq!(state.selected_path, None, "stale: not applied");
        assert!(state.loading, "the newer job is still running");

        app.handle_git_review_updated(kit, 2, Ok(view("second.weapon")));
        let state = &app.kits[0].git_review;
        assert_eq!(state.selected_path.as_deref(), Some("second.weapon"));
        assert!(!state.loading);
    }

    /// Opening a reviewed file finds the entry the kit's own scan made, by
    /// key, rather than adding a second one for the same file.
    #[test]
    fn opening_a_reviewed_file_finds_the_scanned_entry() {
        let root = std::env::temp_dir().join(format!("baboon-git-open-{}", uuid::Uuid::new_v4()));
        let tags = root.join("tags");
        let path = tags.join("objects").join("rifle.weapon");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = vec![0u8; 64];
        bytes[36..40].copy_from_slice(b"weap");
        fs::write(&path, &bytes).unwrap();
        let entry = TagEntry {
            key: format!("file:{}", path.display()),
            display_path: "objects/rifle.weapon".to_owned(),
            group_tag: u32::from_be_bytes(*b"weap"),
            group_name: Some("weapon".to_owned()),
            location: TagEntryLocation::LooseFile(path.clone()),
        };
        let entries = vec![entry.clone()];

        let mut app = Baboon::for_test();
        app.kits[0].source = Some(LoadedSourceData {
            label: "test".to_owned(),
            source: TagSource::LooseFolder {
                root: tags.clone(),
                game: None,
                definitions_root: PathBuf::new(),
            },
            names: Default::default(),
            game: None,
            tree: crate::source::build_tree(&entries),
            group_tree: crate::source::build_tree(&entries),
            entries,
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
        });
        app.kits[0].git_review.repo_root = Some(root.clone());
        let generation = app.kits[0].generation;

        app.open_git_review_file(0, "tags/objects/rifle.weapon");
        let _ = fs::remove_dir_all(&root);

        assert_eq!(app.kits[0].git_review.pending_open, Some(entry.key));
        assert_eq!(app.kits[0].generation, generation, "no entry was added");
        assert_eq!(app.kits[0].source.as_ref().unwrap().entries.len(), 1);
    }
}
